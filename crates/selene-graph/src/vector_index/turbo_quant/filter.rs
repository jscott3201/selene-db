use rayon::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{CoreResult, TURBO_QUANT_BLOCK_ROWS, VectorTopK, VectorValue};
use wide::f64x4;

use super::{TurboQuantVectorHit, TurboQuantVectorIndex, merge_candidate_top_k};

const FILTERED_SLOT_SCAN_MIN_ALLOWED_RATIO: usize = 4;

impl TurboQuantVectorIndex {
    pub(crate) fn candidates_in_rows(
        &self,
        query: &VectorValue,
        k: usize,
        search_width: usize,
        allowed_rows: &RoaringBitmap,
    ) -> CoreResult<Vec<TurboQuantVectorHit>> {
        if k == 0 || self.live_entries == 0 || allowed_rows.is_empty() {
            return Ok(Vec::new());
        }
        let candidate_limit = self.filtered_candidate_limit(k, search_width, allowed_rows);
        if candidate_limit == 0 {
            return Ok(Vec::new());
        }
        let rotated_query = super::rotated_unit_vector(query, self.dimension);
        let query_bias = super::query_bias(&rotated_query, &self.shift);
        let candidates = if self.should_scan_filtered_by_slot_order(allowed_rows) {
            self.slot_order_candidates_fast_scan_in_rows(
                &rotated_query,
                query_bias,
                candidate_limit,
                allowed_rows,
            )
            .unwrap_or_else(|| {
                let byte_lut = self.byte_lut(&rotated_query);
                self.slot_order_candidates_in_rows(
                    &byte_lut,
                    query_bias,
                    candidate_limit,
                    allowed_rows,
                )
            })
        } else {
            let byte_lut = self.byte_lut(&rotated_query);
            self.live_map_candidates_in_rows(&byte_lut, query_bias, candidate_limit, allowed_rows)
        };

        Ok(candidates
            .into_hits()
            .into_iter()
            .map(|hit| TurboQuantVectorHit {
                row: hit.key.1,
                distance: hit.distance,
            })
            .collect())
    }

    pub(crate) fn candidates_batch_in_rows(
        &self,
        queries: &[VectorValue],
        k: usize,
        search_width: usize,
        allowed_rows: &[RoaringBitmap],
    ) -> CoreResult<Vec<Vec<TurboQuantVectorHit>>> {
        debug_assert_eq!(queries.len(), allowed_rows.len());
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if k == 0 || self.live_entries == 0 {
            return Ok(vec![Vec::new(); queries.len()]);
        }
        if !self.should_fuse_filtered_batch_scan(queries.len(), allowed_rows) {
            return queries
                .iter()
                .zip(allowed_rows)
                .map(|(query, allowed)| self.candidates_in_rows(query, k, search_width, allowed))
                .collect();
        }

        let candidate_limits = allowed_rows
            .iter()
            .map(|allowed| self.filtered_candidate_limit(k, search_width, allowed))
            .collect::<Vec<_>>();
        let Some(prepared) = self.prepare_fast_scan_queries(queries) else {
            return queries
                .iter()
                .zip(allowed_rows)
                .map(|(query, allowed)| self.candidates_in_rows(query, k, search_width, allowed))
                .collect();
        };
        let candidates = self.slot_order_candidates_fast_scan_batch_in_rows(
            &prepared,
            &candidate_limits,
            allowed_rows,
        );

        Ok(candidates
            .into_iter()
            .map(|query_candidates| {
                query_candidates
                    .into_hits()
                    .into_iter()
                    .map(|hit| TurboQuantVectorHit {
                        row: hit.key.1,
                        distance: hit.distance,
                    })
                    .collect()
            })
            .collect())
    }

    pub(super) fn block_has_allowed_rows(
        &self,
        block: usize,
        block_len: usize,
        allowed_rows: &RoaringBitmap,
    ) -> bool {
        let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
        (0..block_len).any(|lane| {
            self.allowed_live_row_at_slot(base_slot + lane, allowed_rows)
                .is_some()
        })
    }

    pub(super) fn allowed_live_row_at_slot(
        &self,
        slot: usize,
        allowed_rows: &RoaringBitmap,
    ) -> Option<u32> {
        let row = self.live_row_at_slot(slot)?;
        if !allowed_rows.contains(row) {
            return None;
        }
        Some(row)
    }

    fn filtered_candidate_limit(
        &self,
        k: usize,
        search_width: usize,
        allowed_rows: &RoaringBitmap,
    ) -> usize {
        let allowed_count = usize::try_from(allowed_rows.len()).unwrap_or(usize::MAX);
        search_width
            .max(k)
            .min(self.live_entries)
            .min(allowed_count)
    }

    fn should_scan_filtered_by_slot_order(&self, allowed_rows: &RoaringBitmap) -> bool {
        let allowed_count = usize::try_from(allowed_rows.len()).unwrap_or(usize::MAX);
        self.should_scan_by_slot_order()
            && allowed_count.saturating_mul(FILTERED_SLOT_SCAN_MIN_ALLOWED_RATIO)
                >= self.live_entries
    }

    fn should_fuse_filtered_batch_scan(
        &self,
        query_count: usize,
        allowed_rows: &[RoaringBitmap],
    ) -> bool {
        query_count > 1
            && self.supports_fast_scan_accumulator()
            && self.should_scan_by_slot_order()
            && allowed_rows
                .iter()
                .any(|allowed| self.should_scan_filtered_by_slot_order(allowed))
    }

    pub(super) fn block_has_any_allowed_rows(
        &self,
        block: usize,
        block_len: usize,
        allowed_rows: &[RoaringBitmap],
    ) -> bool {
        allowed_rows
            .iter()
            .any(|allowed| self.block_has_allowed_rows(block, block_len, allowed))
    }

    pub(super) fn slot_order_candidates_in_rows(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> VectorTopK<(usize, u32)> {
        if self.should_parallelize_slot_scan(candidate_limit) {
            return self.slot_order_candidates_in_rows_parallel(
                byte_lut,
                query_bias,
                candidate_limit,
                allowed_rows,
            );
        }
        self.slot_order_candidates_in_rows_blocks(
            0,
            self.codes.block_count(),
            byte_lut,
            query_bias,
            candidate_limit,
            allowed_rows,
        )
    }

    fn slot_order_candidates_in_rows_parallel(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> VectorTopK<(usize, u32)> {
        let chunk_blocks =
            super::TURBO_QUANT_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_in_rows_blocks(
                    start,
                    end,
                    byte_lut,
                    query_bias,
                    candidate_limit,
                    allowed_rows,
                )
            })
            .reduce(|| VectorTopK::new(candidate_limit), merge_candidate_top_k)
    }

    fn slot_order_candidates_in_rows_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> VectorTopK<(usize, u32)> {
        let mut candidates = VectorTopK::new(candidate_limit);
        let mut dots = [f64x4::ZERO; TURBO_QUANT_BLOCK_ROWS / 4];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            if !self.block_has_allowed_rows(block, block_len, allowed_rows) {
                continue;
            }
            dots.fill(f64x4::splat(query_bias));
            for byte in 0..self.bytes_per_row {
                let lut_base = byte * 256;
                let codes = self.codes.block_byte(block, byte);
                for lane_base in (0..TURBO_QUANT_BLOCK_ROWS).step_by(4) {
                    dots[lane_base / 4] += f64x4::from([
                        byte_lut[lut_base + usize::from(codes[lane_base])],
                        byte_lut[lut_base + usize::from(codes[lane_base + 1])],
                        byte_lut[lut_base + usize::from(codes[lane_base + 2])],
                        byte_lut[lut_base + usize::from(codes[lane_base + 3])],
                    ]);
                }
            }
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane_base in (0..block_len).step_by(4) {
                let dot_lanes: [f64; 4] = dots[lane_base / 4].into();
                let active_lanes = (block_len - lane_base).min(4);
                for (lane_offset, dot) in dot_lanes.into_iter().take(active_lanes).enumerate() {
                    let slot = base_slot + lane_base + lane_offset;
                    let Some(row) = self.allowed_live_row_at_slot(slot, allowed_rows) else {
                        continue;
                    };
                    let distance = -(dot * f64::from(self.row_scales[slot]));
                    candidates.push_distance((slot, row), distance);
                }
            }
        }
        candidates
    }

    pub(super) fn live_map_candidates_in_rows(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> VectorTopK<(usize, u32)> {
        let mut candidates = VectorTopK::new(candidate_limit);
        for row in allowed_rows.iter() {
            let Some(slot) = self.slot_for_row(row) else {
                continue;
            };
            let Some(stored_row) = self.rows.get(slot).copied() else {
                continue;
            };
            if stored_row != row {
                continue;
            }
            let distance = self.approx_distance_lut(slot, byte_lut, query_bias);
            candidates.push_distance((slot, row), distance);
        }
        candidates
    }
}
