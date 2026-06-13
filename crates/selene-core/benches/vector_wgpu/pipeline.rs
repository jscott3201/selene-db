use crate::vector_wgpu_shader::{
    BLOCK_TOP_K_SHADER, FUSED_BLOCK_TOP_K_SHADER, PARALLEL_BLOCK_TOP_K_SHADER, SCORE_SHADER,
};

pub(crate) struct Pipelines {
    pub(crate) score: wgpu::ComputePipeline,
    pub(crate) score_bind_group: wgpu::BindGroup,
    pub(crate) block_top_k: wgpu::ComputePipeline,
    pub(crate) block_top_k_bind_group: wgpu::BindGroup,
    pub(crate) fused_block_top_k: wgpu::ComputePipeline,
    pub(crate) parallel_block_top_k: wgpu::ComputePipeline,
    pub(crate) fused_block_top_k_bind_group: wgpu::BindGroup,
}

pub(crate) struct PipelineBuffers<'a> {
    pub(crate) query: &'a wgpu::Buffer,
    pub(crate) candidate: &'a wgpu::Buffer,
    pub(crate) norm: &'a wgpu::Buffer,
    pub(crate) output: &'a wgpu::Buffer,
    pub(crate) partial_distance: &'a wgpu::Buffer,
    pub(crate) partial_index: &'a wgpu::Buffer,
    pub(crate) partial_hit: &'a wgpu::Buffer,
    pub(crate) params: &'a wgpu::Buffer,
}

pub(crate) fn build(device: &wgpu::Device, buffers: PipelineBuffers<'_>) -> Pipelines {
    let score_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("selene vector bind group layout"),
        entries: &[
            storage_entry(0, true),
            storage_entry(1, true),
            storage_entry(2, true),
            storage_entry(3, false),
            uniform_entry(4),
        ],
    });
    let score = pipeline(
        device,
        "selene vector cosine pipeline",
        "selene vector cosine shader",
        &score_layout,
        SCORE_SHADER,
    );
    let score_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("selene vector bind group"),
        layout: &score_layout,
        entries: &[
            bind_entry(0, buffers.query),
            bind_entry(1, buffers.candidate),
            bind_entry(2, buffers.norm),
            bind_entry(3, buffers.output),
            bind_entry(4, buffers.params),
        ],
    });

    let block_top_k_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("selene vector block top-k bind group layout"),
        entries: &[
            storage_entry(0, true),
            storage_entry(1, false),
            storage_entry(2, false),
            uniform_entry(3),
        ],
    });
    let block_top_k = pipeline(
        device,
        "selene vector block top-k pipeline",
        "selene vector block top-k shader",
        &block_top_k_layout,
        BLOCK_TOP_K_SHADER,
    );
    let block_top_k_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("selene vector block top-k bind group"),
        layout: &block_top_k_layout,
        entries: &[
            bind_entry(0, buffers.output),
            bind_entry(1, buffers.partial_distance),
            bind_entry(2, buffers.partial_index),
            bind_entry(3, buffers.params),
        ],
    });

    let fused_block_top_k_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("selene vector fused block top-k bind group layout"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
                uniform_entry(4),
            ],
        });
    let fused_block_top_k = pipeline(
        device,
        "selene vector fused block top-k pipeline",
        "selene vector fused block top-k shader",
        &fused_block_top_k_layout,
        FUSED_BLOCK_TOP_K_SHADER,
    );
    let parallel_block_top_k = pipeline(
        device,
        "selene vector parallel block top-k pipeline",
        "selene vector parallel block top-k shader",
        &fused_block_top_k_layout,
        PARALLEL_BLOCK_TOP_K_SHADER,
    );
    let fused_block_top_k_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("selene vector fused block top-k bind group"),
        layout: &fused_block_top_k_layout,
        entries: &[
            bind_entry(0, buffers.query),
            bind_entry(1, buffers.candidate),
            bind_entry(2, buffers.norm),
            bind_entry(3, buffers.partial_hit),
            bind_entry(4, buffers.params),
        ],
    });

    Pipelines {
        score,
        score_bind_group,
        block_top_k,
        block_top_k_bind_group,
        fused_block_top_k,
        parallel_block_top_k,
        fused_block_top_k_bind_group,
    }
}

fn pipeline(
    device: &wgpu::Device,
    pipeline_label: &'static str,
    shader_label: &'static str,
    bind_group_layout: &wgpu::BindGroupLayout,
    source: &'static str,
) -> wgpu::ComputePipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(pipeline_label),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(shader_label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(pipeline_label),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    })
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
