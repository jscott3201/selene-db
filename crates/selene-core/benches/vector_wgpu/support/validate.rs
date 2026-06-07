use crate::vector_wgpu_case::Case;
use crate::vector_wgpu_fixture::{Fixture, top_k_indices_from_scores};

use super::WgpuBench;
use super::cpu::top_k_indices_from_partials;

impl WgpuBench {
    pub(super) fn assert_matches_cpu(&mut self, fixture: &Fixture) -> Result<(), String> {
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

        self.score_preloaded_fused_block_top_k(&mut partial_distances, &mut partial_indices)?;
        let actual =
            top_k_indices_from_partials(&partial_distances, &partial_indices, self.block_count);
        if actual != expected {
            return Err(format!(
                "fused block top-k drifted: actual={actual:?} expected={expected:?}"
            ));
        }

        self.score_preloaded_parallel_block_top_k(&mut partial_distances, &mut partial_indices)?;
        let actual =
            top_k_indices_from_partials(&partial_distances, &partial_indices, self.block_count);
        if actual != expected {
            return Err(format!(
                "parallel block top-k drifted: actual={actual:?} expected={expected:?}"
            ));
        }
        Ok(())
    }
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
