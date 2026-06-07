use std::sync::mpsc;

use super::WgpuBench;

impl WgpuBench {
    pub(super) fn read_scores(
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

    pub(super) fn read_partials(
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

    pub(super) fn read_packed_partials(
        &self,
        submission: wgpu::SubmissionIndex,
        distances: &mut [f32],
        indices: &mut [u32],
    ) -> Result<(), String> {
        if distances.len() != self.partial_count || indices.len() != self.partial_count {
            return Err("partial output buffers have wrong length".to_string());
        }
        let (tx, rx) = mpsc::channel();
        let slice = self.partial_hit_readback_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.poll(submission)?;
        recv_map_result(rx)?;
        let mapped = slice.get_mapped_range();
        fill_partial_hits(distances, indices, &mapped);
        drop(mapped);
        self.partial_hit_readback_buffer.unmap();
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

fn fill_partial_hits(distances: &mut [f32], indices: &mut [u32], bytes: &[u8]) {
    for ((distance, index), chunk) in distances
        .iter_mut()
        .zip(indices.iter_mut())
        .zip(bytes.chunks_exact(8))
    {
        *distance = f32::from_ne_bytes(chunk[0..4].try_into().expect("chunk is four bytes"));
        *index = u32::from_ne_bytes(chunk[4..8].try_into().expect("chunk is four bytes"));
    }
}
