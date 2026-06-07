#![allow(missing_docs)]
//! Benchmark-only wgpu prototype for batched vector scoring.
//!
//! This is not a production accelerator. It measures the first realistic GPU
//! envelope: candidate vectors stay resident, query batches may be rewritten,
//! a compute shader scores every query/candidate pair, and all scores are read
//! back to the host.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{hint::black_box, sync::mpsc};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::VectorTopK;
use wgpu::util::DeviceExt;

const TOP_K: usize = 10;

const CASES: &[Case] = &[
    Case {
        queries: 8,
        candidates: 4096,
        dimension: 1024,
    },
    Case {
        queries: 16,
        candidates: 4096,
        dimension: 1024,
    },
];

const SHADER: &str = r#"
struct Params {
    queries: u32,
    candidates: u32,
    dimension: u32,
    _padding: u32,
};

@group(0) @binding(0) var<storage, read> queries: array<f32>;
@group(0) @binding(1) var<storage, read> candidates: array<f32>;
@group(0) @binding(2) var<storage, read> norms: array<f32>;
@group(0) @binding(3) var<storage, read_write> distances: array<f32>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let score_index = id.x;
    let total = params.queries * params.candidates;
    if (score_index >= total) {
        return;
    }

    let query_index = score_index / params.candidates;
    let candidate_index = score_index % params.candidates;
    let query_offset = query_index * params.dimension;
    let candidate_offset = candidate_index * params.dimension;

    var dot = 0.0;
    for (var dim = 0u; dim < params.dimension; dim = dim + 1u) {
        dot = dot + queries[query_offset + dim] * candidates[candidate_offset + dim];
    }

    let denom = sqrt(norms[query_index]) * sqrt(norms[params.queries + candidate_index]);
    let similarity = clamp(dot / denom, -1.0, 1.0);
    var distance = 1.0 - similarity;
    if (distance == 0.0) {
        distance = 0.0;
    }
    distances[score_index] = distance;
}
"#;

#[derive(Clone, Copy)]
struct Case {
    queries: usize,
    candidates: usize,
    dimension: usize,
}

impl Case {
    fn id(self, name: &str) -> BenchmarkId {
        BenchmarkId::new(
            name,
            format!("q{}x{}x{}", self.queries, self.candidates, self.dimension),
        )
    }

    const fn score_count(self) -> usize {
        self.queries * self.candidates
    }

    const fn output_bytes(self) -> u64 {
        (self.score_count() * size_of::<f32>()) as u64
    }
}

fn bench_config() -> Criterion {
    let (samples, ms) = match std::env::var("SELENE_BENCH_PROFILE").ok().as_deref() {
        Some("full") | Some("stress") => (30usize, 1_500u64),
        _ => (10, 500),
    };
    Criterion::default()
        .sample_size(samples)
        .warm_up_time(std::time::Duration::from_millis(100))
        .measurement_time(std::time::Duration::from_millis(ms))
}

#[allow(clippy::print_stderr)]
fn bench_vector_wgpu(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_vector_wgpu_prototype");
    for &case in CASES {
        let mut bench = match pollster::block_on(WgpuBench::build(case)) {
            Ok(bench) => bench,
            Err(error) => {
                eprintln!(
                    "[core_vector_wgpu_prototype] skipping q{}x{}x{}: {error}",
                    case.queries, case.candidates, case.dimension
                );
                continue;
            }
        };
        let mut scores = vec![0.0f32; case.score_count()];
        group.throughput(Throughput::Elements(case.score_count() as u64));
        group.bench_function(case.id("resident_query_copy_score_readback"), |b| {
            b.iter(|| {
                bench
                    .score_with_query_write(black_box(&mut scores))
                    .expect("wgpu scoring succeeds")
            });
        });
        group.bench_function(case.id("resident_preloaded_score_readback"), |b| {
            b.iter(|| {
                bench
                    .score_preloaded(black_box(&mut scores))
                    .expect("wgpu scoring succeeds")
            });
        });
        group.bench_function(case.id("cold_candidate_upload_score_readback"), |b| {
            b.iter(|| {
                bench
                    .score_with_candidate_upload(black_box(&mut scores))
                    .expect("wgpu scoring succeeds")
            });
        });
        group.bench_function(
            case.id("resident_query_copy_score_readback_cpu_topk"),
            |b| {
                b.iter(|| {
                    bench
                        .score_with_query_write_top_k(black_box(&mut scores))
                        .expect("wgpu scoring succeeds")
                });
            },
        );
    }
    group.finish();
}

struct WgpuBench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    query_buffer: wgpu::Buffer,
    candidate_buffer: wgpu::Buffer,
    output_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    query_bytes: Vec<u8>,
    candidate_bytes: Vec<u8>,
    candidate_count: usize,
    output_bytes: u64,
    workgroups: u32,
}

impl WgpuBench {
    async fn build(case: Case) -> Result<Self, String> {
        let fixture = Fixture::build(case);
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|error| format!("request adapter failed: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("selene vector wgpu prototype"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("request device failed: {error}"))?;

        let query_bytes = f32_bytes(&fixture.queries);
        let query_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selene vector queries"),
            contents: &query_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let candidate_bytes = f32_bytes(&fixture.candidates);
        let candidate_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selene vector candidates"),
            contents: &candidate_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let norm_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selene vector norms"),
            contents: &f32_bytes(&fixture.norms),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selene vector params"),
            contents: &u32_bytes(&[
                case.queries as u32,
                case.candidates as u32,
                case.dimension as u32,
                0,
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selene vector scores"),
            size: case.output_bytes(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selene vector score readback"),
            size: case.output_bytes(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("selene vector bind group layout"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
                uniform_entry(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("selene vector pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("selene vector cosine shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("selene vector cosine pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("selene vector bind group"),
            layout: &bind_group_layout,
            entries: &[
                bind_entry(0, &query_buffer),
                bind_entry(1, &candidate_buffer),
                bind_entry(2, &norm_buffer),
                bind_entry(3, &output_buffer),
                bind_entry(4, &params_buffer),
            ],
        });

        let mut bench = Self {
            device,
            queue,
            pipeline,
            bind_group,
            query_buffer,
            candidate_buffer,
            output_buffer,
            readback_buffer,
            query_bytes,
            candidate_bytes,
            candidate_count: case.candidates,
            output_bytes: case.output_bytes(),
            workgroups: case.score_count().div_ceil(64) as u32,
        };
        bench.assert_matches_cpu(&fixture)?;
        Ok(bench)
    }

    fn score_with_query_write(&mut self, scores: &mut [f32]) -> Result<f32, String> {
        self.queue
            .write_buffer(&self.query_buffer, 0, &self.query_bytes);
        self.score_preloaded(scores)
    }

    fn score_with_candidate_upload(&mut self, scores: &mut [f32]) -> Result<f32, String> {
        self.queue
            .write_buffer(&self.candidate_buffer, 0, &self.candidate_bytes);
        self.score_with_query_write(scores)
    }

    fn score_with_query_write_top_k(&mut self, scores: &mut [f32]) -> Result<usize, String> {
        self.score_with_query_write(scores)?;
        Ok(cpu_top_k_count(scores, self.candidate_count))
    }

    fn score_preloaded(&mut self, scores: &mut [f32]) -> Result<f32, String> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("selene vector score encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("selene vector score pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(self.workgroups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.output_buffer,
            0,
            &self.readback_buffer,
            0,
            self.output_bytes,
        );
        let submission = self.queue.submit(Some(encoder.finish()));
        self.read_scores(submission, scores)?;
        Ok(scores[0] + scores[scores.len() / 2] + scores[scores.len() - 1])
    }

    fn read_scores(
        &self,
        submission: wgpu::SubmissionIndex,
        scores: &mut [f32],
    ) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let slice = self.readback_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| format!("poll failed: {error}"))?;
        rx.recv()
            .map_err(|error| format!("map callback dropped: {error}"))?
            .map_err(|error| format!("map failed: {error}"))?;
        let mapped = slice.get_mapped_range();
        for (score, chunk) in scores.iter_mut().zip(mapped.chunks_exact(4)) {
            *score = f32::from_ne_bytes(chunk.try_into().expect("chunk is four bytes"));
        }
        drop(mapped);
        self.readback_buffer.unmap();
        Ok(())
    }

    fn assert_matches_cpu(&mut self, fixture: &Fixture) -> Result<(), String> {
        let mut scores = vec![0.0f32; fixture.case.score_count()];
        self.score_preloaded(&mut scores)?;
        let sample_indices = [
            0,
            fixture.case.candidates - 1,
            fixture.case.candidates,
            fixture.case.score_count() / 2,
            fixture.case.score_count() - 1,
        ];
        for idx in sample_indices {
            let gpu = scores[idx];
            let cpu = fixture.cpu_scores[idx];
            let delta = (gpu - cpu).abs();
            if delta > 0.000_01 {
                return Err(format!(
                    "score {idx} drifted: gpu={gpu} cpu={cpu} delta={delta}"
                ));
            }
        }
        Ok(())
    }
}

struct Fixture {
    case: Case,
    queries: Vec<f32>,
    candidates: Vec<f32>,
    norms: Vec<f32>,
    cpu_scores: Vec<f32>,
}

impl Fixture {
    fn build(case: Case) -> Self {
        let queries = flatten_seeded(case.queries, case.dimension, 0);
        let candidates = flatten_seeded(case.candidates, case.dimension, 1_000);
        let query_norms = norms(&queries, case.dimension);
        let candidate_norms = norms(&candidates, case.dimension);
        let cpu_scores = cpu_scores(case, &queries, &candidates, &query_norms, &candidate_norms);
        let norms = query_norms.into_iter().chain(candidate_norms).collect();
        Self {
            case,
            queries,
            candidates,
            norms,
            cpu_scores,
        }
    }
}

fn flatten_seeded(count: usize, dimension: usize, seed_base: usize) -> Vec<f32> {
    let mut vectors = Vec::with_capacity(count * dimension);
    for vector_idx in 0..count {
        vectors.extend(vector_components_seeded(dimension, vector_idx + seed_base));
    }
    vectors
}

fn vector_components_seeded(dim: usize, seed: usize) -> impl Iterator<Item = f32> {
    (0..dim).map(move |idx| (((idx * 31 + seed * 17) % 1_021) as f32 - 510.0) / 256.0)
}

fn norms(vectors: &[f32], dimension: usize) -> Vec<f32> {
    vectors
        .chunks_exact(dimension)
        .map(|vector| vector.iter().map(|component| component * component).sum())
        .collect()
}

fn cpu_scores(
    case: Case,
    queries: &[f32],
    candidates: &[f32],
    query_norms: &[f32],
    candidate_norms: &[f32],
) -> Vec<f32> {
    let mut scores = Vec::with_capacity(case.score_count());
    for (query_idx, &query_norm) in query_norms.iter().enumerate().take(case.queries) {
        let query = window(queries, query_idx, case.dimension);
        for (candidate_idx, &candidate_norm) in
            candidate_norms.iter().enumerate().take(case.candidates)
        {
            let candidate = window(candidates, candidate_idx, case.dimension);
            let dot: f32 = query
                .iter()
                .zip(candidate)
                .map(|(lhs, rhs)| lhs * rhs)
                .sum();
            let denom = query_norm.sqrt() * candidate_norm.sqrt();
            scores.push(1.0 - (dot / denom).clamp(-1.0, 1.0));
        }
    }
    scores
}

fn window(slab: &[f32], index: usize, dimension: usize) -> &[f32] {
    let start = index * dimension;
    &slab[start..start + dimension]
}

fn cpu_top_k_count(scores: &[f32], candidate_count: usize) -> usize {
    let mut retained = 0;
    for query_scores in scores.chunks_exact(candidate_count) {
        let mut top_k = VectorTopK::new(TOP_K);
        for (candidate_idx, &distance) in query_scores.iter().enumerate() {
            top_k.push_distance(candidate_idx, f64::from(distance));
        }
        retained += top_k.into_hits().len();
    }
    retained
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bind_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

criterion_group! {
    name = vector_wgpu;
    config = bench_config();
    targets = bench_vector_wgpu
}
criterion_main!(vector_wgpu);
