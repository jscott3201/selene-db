use rayon::prelude::*;
use roaring::RoaringBitmap;
use selene_core::TURBO_QUANT_BLOCK_ROWS;
use wide::u16x16;

use super::{
    FilteredBatchLaneScratch, PreparedFastScanQuery, fast_scan_candidate_top_k_batch,
    merge_fast_scan_candidate_top_k_batch,
};
use crate::vector_index::turbo_quant::{TurboQuantCandidateTopK, TurboQuantVectorIndex};

impl TurboQuantVectorIndex {
    pub(in crate::vector_index::turbo_quant) fn slot_order_candidates_fast_scan_batch_in_shared_rows(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> Vec<TurboQuantCandidateTopK> {
        if candidate_limit == 0 {
            return fast_scan_candidate_top_k_batch(queries.len(), candidate_limit);
        }
        if self.should_parallelize_slot_scan(candidate_limit) {
            return self.slot_order_candidates_fast_scan_batch_in_shared_rows_parallel(
                queries,
                candidate_limit,
                allowed_rows,
            );
        }
        self.slot_order_candidates_fast_scan_batch_in_shared_rows_blocks(
            0,
            self.codes.block_count(),
            queries,
            candidate_limit,
            allowed_rows,
        )
    }

    fn slot_order_candidates_fast_scan_batch_in_shared_rows_parallel(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> Vec<TurboQuantCandidateTopK> {
        let chunk_blocks = self.parallel_chunk_blocks();
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks)
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_fast_scan_batch_in_shared_rows_blocks(
                    start,
                    end,
                    queries,
                    candidate_limit,
                    allowed_rows,
                )
            })
            .reduce(
                || fast_scan_candidate_top_k_batch(queries.len(), candidate_limit),
                merge_fast_scan_candidate_top_k_batch,
            )
    }

    fn slot_order_candidates_fast_scan_batch_in_shared_rows_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> Vec<TurboQuantCandidateTopK> {
        let mut candidates = fast_scan_candidate_top_k_batch(queries.len(), candidate_limit);
        let mut accumulators = vec![[u16x16::splat(0), u16x16::splat(0)]; queries.len()];
        let mut accumulator_lanes = vec![[[0_i32; 16], [0_i32; 16]]; queries.len()];
        let mut lane_scratch = FilteredBatchLaneScratch::new(queries.len());
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            if !self.shared_filtered_batch_lane_masks(
                block,
                block_len,
                allowed_rows,
                &mut lane_scratch,
            ) {
                continue;
            }
            self.accumulate_fast_scan_batch_block(
                block,
                queries,
                &mut accumulators,
                &mut accumulator_lanes,
            );
            for (((candidate, query), lanes), lane_mask) in candidates
                .iter_mut()
                .zip(queries)
                .zip(accumulator_lanes.iter())
                .zip(lane_scratch.query_lane_masks.iter().copied())
            {
                let mut mask = lane_mask;
                while mask != 0 {
                    let lane = mask.trailing_zeros() as usize;
                    let centered = lanes[lane / 16][lane % 16] - query.lut.zero_sum;
                    let dot = query.query_bias + f64::from(centered) * query.lut.dequant;
                    candidate.push_distance(
                        lane_scratch.lane_rows[lane],
                        -(dot * lane_scratch.lane_scales[lane]),
                    );
                    mask &= mask - 1;
                }
            }
        }
        candidates
    }

    fn shared_filtered_batch_lane_masks(
        &self,
        block: usize,
        block_len: usize,
        allowed_rows: &RoaringBitmap,
        scratch: &mut FilteredBatchLaneScratch,
    ) -> bool {
        scratch.query_lane_masks.fill(0);
        let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
        let mut lane_mask = 0_u32;
        for lane in 0..block_len {
            let slot = base_slot + lane;
            let Some(row) = self.live_row_at_slot(slot) else {
                continue;
            };
            if allowed_rows.contains(row) {
                scratch.lane_rows[lane] = row;
                scratch.lane_scales[lane] = f64::from(self.row_scales[slot]);
                lane_mask |= 1_u32 << lane;
            }
        }
        if lane_mask == 0 {
            return false;
        }
        scratch.query_lane_masks.fill(lane_mask);
        true
    }
}
