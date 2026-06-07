use std::sync::mpsc;

use selene_core::VectorTopK;
use wgpu::util::DeviceExt;

use crate::vector_wgpu_case::{Case, TOP_K};
use crate::vector_wgpu_fixture::{Fixture, top_k_indices_from_scores};
use crate::vector_wgpu_shader::{BLOCK_TOP_K_SHADER, SCORE_SHADER};

pub(crate) struct WgpuBench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    block_top_k_pipeline: wgpu::ComputePipeline,
    block_top_k_bind_group: wgpu::BindGroup,
    query_buffer: wgpu::Buffer,
    candidate_buffer: wgpu::Buffer,
    output_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    partial_distance_buffer: wgpu::Buffer,
    partial_index_buffer: wgpu::Buffer,
    partial_distance_readback_buffer: wgpu::Buffer,
    partial_index_readback_buffer: wgpu::Buffer,
    query_bytes: Vec<u8>,
    candidate_bytes: Vec<u8>,
    candidate_count: usize,
    block_count: usize,
    partial_count: usize,
    partial_f32_bytes: u64,
    partial_u32_bytes: u64,
    output_bytes: u64,
    workgroups: u32,
}

impl WgpuBench {
    pub(crate) async fn build(case: Case) -> Result<Self, String> {
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
        let required_limits = required_limits(case, adapter.limits())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("selene vector wgpu prototype"),
                required_features: wgpu::Features::empty(),
                required_limits,
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
        let output_buffer = create_buffer(
            &device,
            "selene vector scores",
            case.output_bytes(),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback_buffer = create_buffer(
            &device,
            "selene vector score readback",
            case.output_bytes(),
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let partial_distance_buffer = create_buffer(
            &device,
            "selene vector partial top-k distances",
            case.partial_f32_bytes(),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let partial_index_buffer = create_buffer(
            &device,
            "selene vector partial top-k indices",
            case.partial_u32_bytes(),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let partial_distance_readback_buffer = create_buffer(
            &device,
            "selene vector partial distance readback",
            case.partial_f32_bytes(),
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let partial_index_readback_buffer = create_buffer(
            &device,
            "selene vector partial index readback",
            case.partial_u32_bytes(),
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );

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
        let (pipeline, bind_group) = score_pipeline(
            &device,
            &bind_group_layout,
            [
                &query_buffer,
                &candidate_buffer,
                &norm_buffer,
                &output_buffer,
            ],
            &params_buffer,
        );
        let block_top_k_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("selene vector block top-k bind group layout"),
                entries: &[
                    storage_entry(0, true),
                    storage_entry(1, false),
                    storage_entry(2, false),
                    uniform_entry(3),
                ],
            });
        let (block_top_k_pipeline, block_top_k_bind_group) = block_top_k_pipeline(
            &device,
            &block_top_k_bind_group_layout,
            [
                &output_buffer,
                &partial_distance_buffer,
                &partial_index_buffer,
            ],
            &params_buffer,
        );

        let mut bench = Self {
            device,
            queue,
            pipeline,
            bind_group,
            block_top_k_pipeline,
            block_top_k_bind_group,
            query_buffer,
            candidate_buffer,
            output_buffer,
            readback_buffer,
            partial_distance_buffer,
            partial_index_buffer,
            partial_distance_readback_buffer,
            partial_index_readback_buffer,
            query_bytes,
            candidate_bytes,
            candidate_count: case.candidates,
            block_count: case.block_count(),
            partial_count: case.partial_count(),
            partial_f32_bytes: case.partial_f32_bytes(),
            partial_u32_bytes: case.partial_u32_bytes(),
            output_bytes: case.output_bytes(),
            workgroups: case.score_count().div_ceil(64) as u32,
        };
        bench.assert_matches_cpu(&fixture)?;
        Ok(bench)
    }

    pub(crate) fn score_with_query_write(&mut self, scores: &mut [f32]) -> Result<f32, String> {
        self.queue
            .write_buffer(&self.query_buffer, 0, &self.query_bytes);
        self.score_preloaded(scores)
    }

    pub(crate) fn score_with_candidate_upload(
        &mut self,
        scores: &mut [f32],
    ) -> Result<f32, String> {
        self.queue
            .write_buffer(&self.candidate_buffer, 0, &self.candidate_bytes);
        self.score_with_query_write(scores)
    }

    pub(crate) fn score_with_query_write_top_k(
        &mut self,
        scores: &mut [f32],
    ) -> Result<usize, String> {
        self.score_with_query_write(scores)?;
        Ok(cpu_top_k_count(scores, self.candidate_count))
    }

    pub(crate) fn score_with_query_write_block_top_k(
        &mut self,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<usize, String> {
        self.queue
            .write_buffer(&self.query_buffer, 0, &self.query_bytes);
        self.score_preloaded_block_top_k(distances, indices)
    }

    pub(crate) fn score_preloaded(&mut self, scores: &mut [f32]) -> Result<f32, String> {
        let mut encoder = self.device.create_command_encoder(&encoder_desc("score"));
        self.encode_score_pass(&mut encoder);
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

    fn score_preloaded_block_top_k(
        &mut self,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<usize, String> {
        let mut encoder = self
            .device
            .create_command_encoder(&encoder_desc("block top-k"));
        self.encode_score_pass(&mut encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("selene vector block top-k pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.block_top_k_pipeline);
            pass.set_bind_group(0, &self.block_top_k_bind_group, &[]);
            pass.dispatch_workgroups(self.block_count as u32, self.query_count(), 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.partial_distance_buffer,
            0,
            &self.partial_distance_readback_buffer,
            0,
            self.partial_f32_bytes,
        );
        encoder.copy_buffer_to_buffer(
            &self.partial_index_buffer,
            0,
            &self.partial_index_readback_buffer,
            0,
            self.partial_u32_bytes,
        );
        let submission = self.queue.submit(Some(encoder.finish()));
        self.read_partials(submission, distances, indices)?;
        Ok(cpu_merge_partial_top_k_count(
            distances,
            indices,
            self.block_count,
        ))
    }

    fn encode_score_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("selene vector score pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(self.workgroups, 1, 1);
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
        self.poll(submission)?;
        recv_map_result(rx)?;
        let mapped = slice.get_mapped_range();
        fill_f32(scores, &mapped);
        drop(mapped);
        self.readback_buffer.unmap();
        Ok(())
    }

    fn read_partials(
        &self,
        submission: wgpu::SubmissionIndex,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<(), String> {
        if distances.len() != self.partial_count || indices.len() != self.partial_count {
            return Err("partial output buffers have wrong length".to_string());
        }
        let (distance_tx, distance_rx) = mpsc::channel();
        let (index_tx, index_rx) = mpsc::channel();
        let distance_slice = self.partial_distance_readback_buffer.slice(..);
        let index_slice = self.partial_index_readback_buffer.slice(..);
        distance_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = distance_tx.send(result);
        });
        index_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = index_tx.send(result);
        });
        self.poll(submission)?;
        recv_map_result(distance_rx)?;
        recv_map_result(index_rx)?;
        let mapped_distances = distance_slice.get_mapped_range();
        let mapped_indices = index_slice.get_mapped_range();
        fill_f32(distances, &mapped_distances);
        fill_u32(indices, &mapped_indices);
        drop(mapped_indices);
        drop(mapped_distances);
        self.partial_index_readback_buffer.unmap();
        self.partial_distance_readback_buffer.unmap();
        Ok(())
    }

    fn assert_matches_cpu(&mut self, fixture: &Fixture) -> Result<(), String> {
        let mut scores = vec![0.0f32; fixture.case.score_count()];
        self.score_preloaded(&mut scores)?;
        for idx in sample_indices(fixture.case) {
            let gpu = scores[idx];
            let cpu = fixture.cpu_scores[idx];
            let delta = (gpu - cpu).abs();
            if delta > 0.000_01 {
                return Err(format!(
                    "score {idx} drifted: gpu={gpu} cpu={cpu} delta={delta}"
                ));
            }
        }
        let mut partial_distances = vec![0.0f32; self.partial_count];
        let mut partial_indices = vec![0u32; self.partial_count];
        self.score_preloaded_block_top_k(&mut partial_distances, &mut partial_indices)?;
        let expected = top_k_indices_from_scores(&scores, self.candidate_count);
        let actual =
            top_k_indices_from_partials(&partial_distances, &partial_indices, self.block_count);
        if actual != expected {
            return Err(format!(
                "block top-k drifted: actual={actual:?} expected={expected:?}"
            ));
        }
        Ok(())
    }

    fn poll(&self, submission: wgpu::SubmissionIndex) -> Result<(), String> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| format!("poll failed: {error}"))?;
        Ok(())
    }

    fn query_count(&self) -> u32 {
        (self.partial_count / (self.block_count * TOP_K)) as u32
    }
}

fn score_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 4],
    params_buffer: &wgpu::Buffer,
) -> (wgpu::ComputePipeline, wgpu::BindGroup) {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("selene vector pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("selene vector cosine shader"),
        source: wgpu::ShaderSource::Wgsl(SCORE_SHADER.into()),
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
        layout: bind_group_layout,
        entries: &[
            bind_entry(0, buffers[0]),
            bind_entry(1, buffers[1]),
            bind_entry(2, buffers[2]),
            bind_entry(3, buffers[3]),
            bind_entry(4, params_buffer),
        ],
    });
    (pipeline, bind_group)
}

fn block_top_k_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 3],
    params_buffer: &wgpu::Buffer,
) -> (wgpu::ComputePipeline, wgpu::BindGroup) {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("selene vector block top-k pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("selene vector block top-k shader"),
        source: wgpu::ShaderSource::Wgsl(BLOCK_TOP_K_SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("selene vector block top-k pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("selene vector block top-k bind group"),
        layout: bind_group_layout,
        entries: &[
            bind_entry(0, buffers[0]),
            bind_entry(1, buffers[1]),
            bind_entry(2, buffers[2]),
            bind_entry(3, params_buffer),
        ],
    });
    (pipeline, bind_group)
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

fn cpu_merge_partial_top_k_count(distances: &[f32], indices: &[u32], block_count: usize) -> usize {
    top_k_indices_from_partials(distances, indices, block_count)
        .into_iter()
        .map(|hits| hits.len())
        .sum()
}

fn top_k_indices_from_partials(
    distances: &[f32],
    indices: &[u32],
    block_count: usize,
) -> Vec<Vec<usize>> {
    let query_width = block_count * TOP_K;
    distances
        .chunks_exact(query_width)
        .zip(indices.chunks_exact(query_width))
        .map(|(query_distances, query_indices)| {
            let mut top_k = VectorTopK::new(TOP_K);
            for (&distance, &candidate_idx) in query_distances.iter().zip(query_indices) {
                if candidate_idx != u32::MAX {
                    top_k.push_distance(candidate_idx as usize, f64::from(distance));
                }
            }
            top_k.into_hits().into_iter().map(|hit| hit.key).collect()
        })
        .collect()
}

fn sample_indices(case: Case) -> [usize; 5] {
    [
        0,
        case.candidates - 1,
        case.candidates,
        case.score_count() / 2,
        case.score_count() - 1,
    ]
}

fn required_limits(case: Case, adapter_limits: wgpu::Limits) -> Result<wgpu::Limits, String> {
    let mut limits = wgpu::Limits::downlevel_defaults();
    let storage_bytes = case.largest_storage_bytes();
    if storage_bytes > adapter_limits.max_storage_buffer_binding_size {
        return Err(format!(
            "case requires {storage_bytes} byte storage binding but adapter supports {}",
            adapter_limits.max_storage_buffer_binding_size
        ));
    }
    if storage_bytes > adapter_limits.max_buffer_size {
        return Err(format!(
            "case requires {storage_bytes} byte buffer but adapter supports {}",
            adapter_limits.max_buffer_size
        ));
    }
    limits.max_storage_buffer_binding_size =
        limits.max_storage_buffer_binding_size.max(storage_bytes);
    limits.max_buffer_size = limits.max_buffer_size.max(storage_bytes);
    Ok(limits)
}

fn create_buffer(
    device: &wgpu::Device,
    label: &'static str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

fn encoder_desc(label: &'static str) -> wgpu::CommandEncoderDescriptor<'static> {
    wgpu::CommandEncoderDescriptor { label: Some(label) }
}

fn recv_map_result(rx: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>) -> Result<(), String> {
    rx.recv()
        .map_err(|error| format!("map callback dropped: {error}"))?
        .map_err(|error| format!("map failed: {error}"))
}

fn fill_f32(output: &mut [f32], bytes: &[u8]) {
    for (value, chunk) in output.iter_mut().zip(bytes.chunks_exact(4)) {
        *value = f32::from_ne_bytes(chunk.try_into().expect("chunk is four bytes"));
    }
}

fn fill_u32(output: &mut [u32], bytes: &[u8]) {
    for (value, chunk) in output.iter_mut().zip(bytes.chunks_exact(4)) {
        *value = u32::from_ne_bytes(chunk.try_into().expect("chunk is four bytes"));
    }
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
