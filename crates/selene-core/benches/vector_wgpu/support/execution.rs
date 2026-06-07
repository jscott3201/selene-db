use super::WgpuBench;
use super::cpu::{cpu_merge_partial_top_k_count, cpu_top_k_count};

impl WgpuBench {
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

    pub(crate) fn score_with_query_write_fused_block_top_k(
        &mut self,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<usize, String> {
        self.queue
            .write_buffer(&self.query_buffer, 0, &self.query_bytes);
        self.score_preloaded_fused_block_top_k(distances, indices)
    }

    pub(crate) fn score_with_query_write_parallel_block_top_k(
        &mut self,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<usize, String> {
        self.queue
            .write_buffer(&self.query_buffer, 0, &self.query_bytes);
        self.score_preloaded_parallel_block_top_k(distances, indices)
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

    pub(super) fn score_preloaded_block_top_k(
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
            pass.set_pipeline(&self.pipelines.block_top_k);
            pass.set_bind_group(0, &self.pipelines.block_top_k_bind_group, &[]);
            pass.dispatch_workgroups(self.block_count as u32, self.query_count(), 1);
        }
        self.copy_partials(&mut encoder);
        let submission = self.queue.submit(Some(encoder.finish()));
        self.read_partials(submission, distances, indices)?;
        Ok(cpu_merge_partial_top_k_count(
            distances,
            indices,
            self.block_count,
        ))
    }

    pub(super) fn score_preloaded_fused_block_top_k(
        &mut self,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<usize, String> {
        let mut encoder = self
            .device
            .create_command_encoder(&encoder_desc("fused block top-k"));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("selene vector fused block top-k pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.fused_block_top_k);
            pass.set_bind_group(0, &self.pipelines.fused_block_top_k_bind_group, &[]);
            pass.dispatch_workgroups(self.block_count as u32, self.query_count(), 1);
        }
        self.copy_packed_partials(&mut encoder);
        let submission = self.queue.submit(Some(encoder.finish()));
        self.read_packed_partials(submission, distances, indices)?;
        Ok(cpu_merge_partial_top_k_count(
            distances,
            indices,
            self.block_count,
        ))
    }

    pub(super) fn score_preloaded_parallel_block_top_k(
        &mut self,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<usize, String> {
        let mut encoder = self
            .device
            .create_command_encoder(&encoder_desc("parallel block top-k"));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("selene vector parallel block top-k pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.parallel_block_top_k);
            pass.set_bind_group(0, &self.pipelines.fused_block_top_k_bind_group, &[]);
            pass.dispatch_workgroups(self.block_count as u32, self.query_count(), 1);
        }
        self.copy_packed_partials(&mut encoder);
        let submission = self.queue.submit(Some(encoder.finish()));
        self.read_packed_partials(submission, distances, indices)?;
        Ok(cpu_merge_partial_top_k_count(
            distances,
            indices,
            self.block_count,
        ))
    }

    fn copy_packed_partials(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_buffer_to_buffer(
            &self.partial_hit_buffer,
            0,
            &self.partial_hit_readback_buffer,
            0,
            self.partial_hit_bytes,
        );
    }

    fn copy_partials(&self, encoder: &mut wgpu::CommandEncoder) {
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
    }

    fn encode_score_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("selene vector score pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipelines.score);
        pass.set_bind_group(0, &self.pipelines.score_bind_group, &[]);
        pass.dispatch_workgroups(self.workgroups, 1, 1);
    }
}

fn encoder_desc(label: &'static str) -> wgpu::CommandEncoderDescriptor<'static> {
    wgpu::CommandEncoderDescriptor { label: Some(label) }
}
