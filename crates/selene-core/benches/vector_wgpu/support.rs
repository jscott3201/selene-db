use crate::vector_wgpu_case::TOP_K;
use crate::vector_wgpu_pipeline::Pipelines;

#[path = "support/build.rs"]
mod build;
#[path = "support/cpu.rs"]
mod cpu;
#[path = "support/execution.rs"]
mod execution;
#[path = "support/readback.rs"]
mod readback;
#[path = "support/validate.rs"]
mod validate;

pub(crate) use cpu::{fixture_parallel_score_top_k, fixture_parallel_score_top_k_hot_shard_reuse};

pub(crate) struct WgpuBench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    query_buffer: wgpu::Buffer,
    candidate_buffer: wgpu::Buffer,
    output_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    partial_distance_buffer: wgpu::Buffer,
    partial_index_buffer: wgpu::Buffer,
    partial_distance_readback_buffer: wgpu::Buffer,
    partial_index_readback_buffer: wgpu::Buffer,
    partial_hit_buffer: wgpu::Buffer,
    partial_hit_readback_buffer: wgpu::Buffer,
    queries: Vec<f32>,
    candidates: Vec<f32>,
    norms: Vec<f32>,
    query_bytes: Vec<u8>,
    candidate_bytes: Vec<u8>,
    dimension: usize,
    candidate_count: usize,
    block_count: usize,
    partial_count: usize,
    partial_f32_bytes: u64,
    partial_u32_bytes: u64,
    partial_hit_bytes: u64,
    output_bytes: u64,
    workgroups: u32,
}

impl WgpuBench {
    pub(super) fn query_count(&self) -> u32 {
        (self.partial_count / (self.block_count * TOP_K)) as u32
    }
}
