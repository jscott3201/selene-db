use wgpu::util::DeviceExt;

use crate::vector_wgpu_case::Case;
use crate::vector_wgpu_fixture::Fixture;
use crate::vector_wgpu_pipeline::PipelineBuffers;

use super::WgpuBench;

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
        let partial_hit_buffer = create_buffer(
            &device,
            "selene vector fused partial hits",
            case.partial_hit_bytes(),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let partial_hit_readback_buffer = create_buffer(
            &device,
            "selene vector fused partial hit readback",
            case.partial_hit_bytes(),
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );

        let pipelines = crate::vector_wgpu_pipeline::build(
            &device,
            PipelineBuffers {
                query: &query_buffer,
                candidate: &candidate_buffer,
                norm: &norm_buffer,
                output: &output_buffer,
                partial_distance: &partial_distance_buffer,
                partial_index: &partial_index_buffer,
                partial_hit: &partial_hit_buffer,
                params: &params_buffer,
            },
        );

        let mut bench = Self {
            device,
            queue,
            pipelines,
            query_buffer,
            candidate_buffer,
            output_buffer,
            readback_buffer,
            partial_distance_buffer,
            partial_index_buffer,
            partial_distance_readback_buffer,
            partial_index_readback_buffer,
            partial_hit_buffer,
            partial_hit_readback_buffer,
            queries: fixture.queries.clone(),
            candidates: fixture.candidates.clone(),
            norms: fixture.norms.clone(),
            query_bytes,
            candidate_bytes,
            dimension: case.dimension,
            candidate_count: case.candidates,
            block_count: case.block_count(),
            partial_count: case.partial_count(),
            partial_f32_bytes: case.partial_f32_bytes(),
            partial_u32_bytes: case.partial_u32_bytes(),
            partial_hit_bytes: case.partial_hit_bytes(),
            output_bytes: case.output_bytes(),
            workgroups: case.score_count().div_ceil(64) as u32,
        };
        bench.assert_matches_cpu(&fixture)?;
        Ok(bench)
    }
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
