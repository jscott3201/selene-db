use std::array;

use rayon::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{TURBO_QUANT_BLOCK_ROWS, VectorValue};
use wide::{i16x8, u8x16, u16x16};

use super::{
    TurboQuantCandidateTopK, TurboQuantVectorIndex, merge_candidate_top_k,
    query_component_for_score,
};

const FAST_SCAN_QUANT_LIMIT: i16 = i8::MAX as i16;
const FAST_SCAN_FLUSH_BYTES: usize = 128;

impl TurboQuantVectorIndex {
    pub(super) fn slot_order_candidates_fast_scan(
        &self,
        rotated_query: &[f32],
        query_bias: f64,
        candidate_limit: usize,
    ) -> Option<TurboQuantCandidateTopK> {
        let lut = self.fast_scan_lut(rotated_query)?;
        if self.should_parallelize_slot_scan(candidate_limit) {
            return Some(self.slot_order_candidates_fast_scan_parallel(
                &lut,
                query_bias,
                candidate_limit,
            ));
        }
        Some(self.slot_order_candidates_fast_scan_blocks(
            0,
            self.codes.block_count(),
            &lut,
            query_bias,
            candidate_limit,
        ))
    }

    pub(super) fn slot_order_candidates_fast_scan_in_rows(
        &self,
        rotated_query: &[f32],
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> Option<TurboQuantCandidateTopK> {
        let lut = self.fast_scan_lut(rotated_query)?;
        if self.should_parallelize_slot_scan(candidate_limit) {
            return Some(self.slot_order_candidates_fast_scan_in_rows_parallel(
                &lut,
                query_bias,
                candidate_limit,
                allowed_rows,
            ));
        }
        Some(self.slot_order_candidates_fast_scan_in_rows_blocks(
            0,
            self.codes.block_count(),
            &lut,
            query_bias,
            candidate_limit,
            allowed_rows,
        ))
    }

    pub(super) fn prepare_fast_scan_queries(
        &self,
        queries: &[VectorValue],
    ) -> Option<Vec<PreparedFastScanQuery>> {
        queries
            .iter()
            .map(|query| {
                let rotated_query = super::rotated_unit_vector(query, self.dimension);
                let query_bias = super::query_bias(&rotated_query, &self.shift);
                self.fast_scan_lut(&rotated_query)
                    .map(|lut| PreparedFastScanQuery { lut, query_bias })
            })
            .collect()
    }

    pub(super) fn slot_order_candidates_fast_scan_batch(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
    ) -> Vec<TurboQuantCandidateTopK> {
        if self.should_parallelize_slot_scan(candidate_limit) {
            return self.slot_order_candidates_fast_scan_batch_parallel(queries, candidate_limit);
        }
        self.slot_order_candidates_fast_scan_batch_blocks(
            0,
            self.codes.block_count(),
            queries,
            candidate_limit,
        )
    }

    pub(super) fn slot_order_candidates_fast_scan_batch_in_rows(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limits: &[usize],
        allowed_rows: &[RoaringBitmap],
    ) -> Vec<TurboQuantCandidateTopK> {
        debug_assert_eq!(queries.len(), candidate_limits.len());
        debug_assert_eq!(queries.len(), allowed_rows.len());
        let max_candidate_limit = candidate_limits.iter().copied().max().unwrap_or_default();
        if max_candidate_limit == 0 {
            return fast_scan_candidate_top_k_batch_with_limits(candidate_limits);
        }
        if self.should_parallelize_slot_scan(max_candidate_limit) {
            return self.slot_order_candidates_fast_scan_batch_in_rows_parallel(
                queries,
                candidate_limits,
                allowed_rows,
            );
        }
        self.slot_order_candidates_fast_scan_batch_in_rows_blocks(
            0,
            self.codes.block_count(),
            queries,
            candidate_limits,
            allowed_rows,
        )
    }

    fn slot_order_candidates_fast_scan_batch_parallel(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
    ) -> Vec<TurboQuantCandidateTopK> {
        let chunk_blocks =
            super::TURBO_QUANT_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_fast_scan_batch_blocks(
                    start,
                    end,
                    queries,
                    candidate_limit,
                )
            })
            .reduce(
                || fast_scan_candidate_top_k_batch(queries.len(), candidate_limit),
                merge_fast_scan_candidate_top_k_batch,
            )
    }

    fn slot_order_candidates_fast_scan_batch_in_rows_parallel(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limits: &[usize],
        allowed_rows: &[RoaringBitmap],
    ) -> Vec<TurboQuantCandidateTopK> {
        let chunk_blocks =
            super::TURBO_QUANT_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_fast_scan_batch_in_rows_blocks(
                    start,
                    end,
                    queries,
                    candidate_limits,
                    allowed_rows,
                )
            })
            .reduce(
                || fast_scan_candidate_top_k_batch_with_limits(candidate_limits),
                merge_fast_scan_candidate_top_k_batch,
            )
    }

    fn slot_order_candidates_fast_scan_batch_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
    ) -> Vec<TurboQuantCandidateTopK> {
        let mut candidates = fast_scan_candidate_top_k_batch(queries.len(), candidate_limit);
        let mut accumulators = vec![[u16x16::splat(0), u16x16::splat(0)]; queries.len()];
        let mut accumulator_lanes = vec![[[0_i32; 16], [0_i32; 16]]; queries.len()];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            self.accumulate_fast_scan_batch_block(
                block,
                queries,
                &mut accumulators,
                &mut accumulator_lanes,
            );
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let slot = base_slot + lane;
                let Some(row) = self.live_row_at_slot(slot) else {
                    continue;
                };
                for ((candidate, query), lanes) in candidates
                    .iter_mut()
                    .zip(queries)
                    .zip(accumulator_lanes.iter())
                {
                    let centered = lanes[lane / 16][lane % 16] - query.lut.zero_sum;
                    let dot = query.query_bias + f64::from(centered) * query.lut.dequant;
                    let distance = -(dot * f64::from(self.row_scales[slot]));
                    candidate.push_distance(row, distance);
                }
            }
        }
        candidates
    }

    fn slot_order_candidates_fast_scan_batch_in_rows_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        queries: &[PreparedFastScanQuery],
        candidate_limits: &[usize],
        allowed_rows: &[RoaringBitmap],
    ) -> Vec<TurboQuantCandidateTopK> {
        let mut candidates = fast_scan_candidate_top_k_batch_with_limits(candidate_limits);
        let mut accumulators = vec![[u16x16::splat(0), u16x16::splat(0)]; queries.len()];
        let mut accumulator_lanes = vec![[[0_i32; 16], [0_i32; 16]]; queries.len()];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            if !self.block_has_any_allowed_rows(block, block_len, allowed_rows) {
                continue;
            }
            self.accumulate_fast_scan_batch_block(
                block,
                queries,
                &mut accumulators,
                &mut accumulator_lanes,
            );
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let slot = base_slot + lane;
                for (((candidate, query), lanes), allowed) in candidates
                    .iter_mut()
                    .zip(queries)
                    .zip(accumulator_lanes.iter())
                    .zip(allowed_rows)
                {
                    let Some(row) = self.allowed_live_row_at_slot(slot, allowed) else {
                        continue;
                    };
                    let centered = lanes[lane / 16][lane % 16] - query.lut.zero_sum;
                    let dot = query.query_bias + f64::from(centered) * query.lut.dequant;
                    let distance = -(dot * f64::from(self.row_scales[slot]));
                    candidate.push_distance(row, distance);
                }
            }
        }
        candidates
    }

    fn slot_order_candidates_fast_scan_parallel(
        &self,
        lut: &FastScanQueryLut,
        query_bias: f64,
        candidate_limit: usize,
    ) -> TurboQuantCandidateTopK {
        let chunk_blocks =
            super::TURBO_QUANT_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_fast_scan_blocks(
                    start,
                    end,
                    lut,
                    query_bias,
                    candidate_limit,
                )
            })
            .reduce(
                || TurboQuantCandidateTopK::new(candidate_limit),
                merge_candidate_top_k,
            )
    }

    fn slot_order_candidates_fast_scan_in_rows_parallel(
        &self,
        lut: &FastScanQueryLut,
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> TurboQuantCandidateTopK {
        let chunk_blocks =
            super::TURBO_QUANT_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_fast_scan_in_rows_blocks(
                    start,
                    end,
                    lut,
                    query_bias,
                    candidate_limit,
                    allowed_rows,
                )
            })
            .reduce(
                || TurboQuantCandidateTopK::new(candidate_limit),
                merge_candidate_top_k,
            )
    }

    fn slot_order_candidates_fast_scan_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        lut: &FastScanQueryLut,
        query_bias: f64,
        candidate_limit: usize,
    ) -> TurboQuantCandidateTopK {
        let mut candidates = TurboQuantCandidateTopK::new(candidate_limit);
        let mut accumulators = [u16x16::splat(0), u16x16::splat(0)];
        let mut lanes = [[0_i32; 16], [0_i32; 16]];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            self.accumulate_fast_scan_block(block, lut, &mut accumulators, &mut lanes);
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let slot = base_slot + lane;
                let Some(row) = self.live_row_at_slot(slot) else {
                    continue;
                };
                let centered = lanes[lane / 16][lane % 16] - lut.zero_sum;
                let dot = query_bias + f64::from(centered) * lut.dequant;
                let distance = -(dot * f64::from(self.row_scales[slot]));
                candidates.push_distance(row, distance);
            }
        }
        candidates
    }

    fn slot_order_candidates_fast_scan_in_rows_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        lut: &FastScanQueryLut,
        query_bias: f64,
        candidate_limit: usize,
        allowed_rows: &RoaringBitmap,
    ) -> TurboQuantCandidateTopK {
        let mut candidates = TurboQuantCandidateTopK::new(candidate_limit);
        let mut accumulators = [u16x16::splat(0), u16x16::splat(0)];
        let mut lanes = [[0_i32; 16], [0_i32; 16]];
        let mut lane_rows = [0_u32; TURBO_QUANT_BLOCK_ROWS];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            let lane_mask = self.filtered_lane_mask(block, block_len, allowed_rows, &mut lane_rows);
            if lane_mask == 0 {
                continue;
            }
            self.accumulate_fast_scan_block(block, lut, &mut accumulators, &mut lanes);
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            let mut mask = lane_mask;
            while mask != 0 {
                let lane = mask.trailing_zeros() as usize;
                let slot = base_slot + lane;
                let row = lane_rows[lane];
                let centered = lanes[lane / 16][lane % 16] - lut.zero_sum;
                let dot = query_bias + f64::from(centered) * lut.dequant;
                let distance = -(dot * f64::from(self.row_scales[slot]));
                candidates.push_distance(row, distance);
                mask &= mask - 1;
            }
        }
        candidates
    }

    fn filtered_lane_mask(
        &self,
        block: usize,
        block_len: usize,
        allowed_rows: &RoaringBitmap,
        lane_rows: &mut [u32; TURBO_QUANT_BLOCK_ROWS],
    ) -> u32 {
        let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
        let mut mask = 0_u32;
        for (lane, lane_row) in lane_rows.iter_mut().enumerate().take(block_len) {
            let Some(row) = self.live_row_at_slot(base_slot + lane) else {
                continue;
            };
            if allowed_rows.contains(row) {
                *lane_row = row;
                mask |= 1_u32 << lane;
            }
        }
        mask
    }

    fn accumulate_fast_scan_block(
        &self,
        block: usize,
        lut: &FastScanQueryLut,
        accumulators: &mut [u16x16; 2],
        lanes: &mut [[i32; 16]; 2],
    ) {
        lanes.fill([0; 16]);
        for byte_start in (0..self.bytes_per_row).step_by(lut.flush_bytes) {
            accumulators.fill(u16x16::splat(0));
            for byte in byte_start..(byte_start + lut.flush_bytes).min(self.bytes_per_row) {
                let byte_lut = &lut.bytes[byte];
                let codes = self.codes.block_byte(block, byte);
                accumulate_half(&mut accumulators[0], load_lanes(&codes[..16]), byte_lut);
                accumulate_half(&mut accumulators[1], load_lanes(&codes[16..]), byte_lut);
            }
            add_accumulator_lanes(lanes, accumulators);
        }
    }

    fn accumulate_fast_scan_batch_block(
        &self,
        block: usize,
        queries: &[PreparedFastScanQuery],
        accumulators: &mut [[u16x16; 2]],
        lanes: &mut [[[i32; 16]; 2]],
    ) {
        lanes.fill([[0; 16], [0; 16]]);
        let flush_bytes = queries
            .first()
            .map(|query| query.lut.flush_bytes)
            .unwrap_or(self.bytes_per_row);
        for byte_start in (0..self.bytes_per_row).step_by(flush_bytes) {
            accumulators.fill([u16x16::splat(0), u16x16::splat(0)]);
            for byte in byte_start..(byte_start + flush_bytes).min(self.bytes_per_row) {
                let codes = self.codes.block_byte(block, byte);
                let low_lanes = load_lanes(&codes[..16]);
                let high_lanes = load_lanes(&codes[16..]);
                for (accumulator, query) in accumulators.iter_mut().zip(queries) {
                    let byte_lut = &query.lut.bytes[byte];
                    accumulate_half(&mut accumulator[0], low_lanes, byte_lut);
                    accumulate_half(&mut accumulator[1], high_lanes, byte_lut);
                }
            }
            for (query_lanes, accumulator) in lanes.iter_mut().zip(accumulators.iter()) {
                add_accumulator_lanes(query_lanes, accumulator);
            }
        }
    }

    pub(super) fn fast_scan_lut(&self, rotated_query: &[f32]) -> Option<FastScanQueryLut> {
        let components = self.fast_scan_components()?;
        let quant_limit = FAST_SCAN_QUANT_LIMIT;
        let max_abs = self.max_fast_scan_query_contribution(rotated_query);
        let quant_scale = if max_abs > 0.0 {
            f64::from(quant_limit) / max_abs
        } else {
            0.0
        };
        let dequant = if quant_scale > 0.0 {
            quant_scale.recip()
        } else {
            0.0
        };
        let bytes = (0..self.bytes_per_row)
            .map(|byte| {
                let first_dim = byte * 2;
                let second_dim = first_dim + 1;
                FastScanByteLut {
                    first: self.dimension_fast_scan_lut(
                        first_dim,
                        rotated_query,
                        quant_scale,
                        quant_limit,
                    ),
                    second: self.dimension_fast_scan_lut(
                        second_dim,
                        rotated_query,
                        quant_scale,
                        quant_limit,
                    ),
                }
            })
            .collect();
        Some(FastScanQueryLut {
            bytes,
            zero_sum: (components as i32) * i32::from(quant_limit),
            dequant,
            flush_bytes: self.bytes_per_row.clamp(1, FAST_SCAN_FLUSH_BYTES),
        })
    }

    pub(super) fn supports_fast_scan_accumulator(&self) -> bool {
        self.fast_scan_components().is_some()
    }

    fn fast_scan_components(&self) -> Option<usize> {
        self.bytes_per_row.checked_mul(2)
    }

    fn max_fast_scan_query_contribution(&self, rotated_query: &[f32]) -> f64 {
        (0..self.dimension)
            .flat_map(|dimension| {
                let query =
                    query_component_for_score(rotated_query[dimension], dimension, &self.inv_scale);
                self.codebook
                    .centroids()
                    .iter()
                    .map(move |centroid| f64::from(query) * f64::from(*centroid))
            })
            .map(f64::abs)
            .fold(0.0, f64::max)
    }

    fn dimension_fast_scan_lut(
        &self,
        dimension: usize,
        rotated_query: &[f32],
        quant_scale: f64,
        quant_limit: i16,
    ) -> u8x16 {
        if dimension >= self.dimension {
            return u8x16::splat(quant_limit as u8);
        }
        let query = query_component_for_score(rotated_query[dimension], dimension, &self.inv_scale);
        let table = array::from_fn(|code| {
            quantized_contribution(
                f64::from(query) * f64::from(self.codebook.centroids()[code]),
                quant_scale,
                quant_limit,
            )
        });
        u8x16::new(table)
    }
}

#[derive(Clone)]
pub(super) struct FastScanQueryLut {
    bytes: Vec<FastScanByteLut>,
    zero_sum: i32,
    dequant: f64,
    flush_bytes: usize,
}

pub(super) struct PreparedFastScanQuery {
    lut: FastScanQueryLut,
    query_bias: f64,
}

#[derive(Clone, Copy)]
struct FastScanByteLut {
    first: u8x16,
    second: u8x16,
}

fn accumulate_half(accumulator: &mut u16x16, codes: u8x16, byte_lut: &FastScanByteLut) {
    let low_codes = codes & u8x16::splat(0x0f);
    let high_codes = high_nibbles(codes);
    let first = byte_lut.first.swizzle_relaxed(low_codes);
    let second = byte_lut.second.swizzle_relaxed(high_codes);
    *accumulator = *accumulator + u16x16::from(first) + u16x16::from(second);
}

fn high_nibbles(codes: u8x16) -> u8x16 {
    let low = (i16x8::from_u8x16_low(codes) >> 4_u8) & i16x8::splat(0x0f);
    let high = (i16x8::from_u8x16_high(codes) >> 4_u8) & i16x8::splat(0x0f);
    u8x16::narrow_i16x8(low, high)
}

fn load_lanes(codes: &[u8]) -> u8x16 {
    let lanes: [u8; 16] = codes
        .try_into()
        .expect("FastScan scorer loads exactly sixteen lanes");
    u8x16::new(lanes)
}

fn add_accumulator_lanes(lanes: &mut [[i32; 16]; 2], accumulators: &[u16x16; 2]) {
    let low = accumulators[0].to_array();
    let high = accumulators[1].to_array();
    for (lane, value) in lanes[0].iter_mut().zip(low) {
        *lane += i32::from(value);
    }
    for (lane, value) in lanes[1].iter_mut().zip(high) {
        *lane += i32::from(value);
    }
}

fn quantized_contribution(value: f64, quant_scale: f64, quant_limit: i16) -> u8 {
    if quant_scale == 0.0 {
        return quant_limit as u8;
    }
    let quantized = (value * quant_scale)
        .round()
        .clamp(-f64::from(quant_limit), f64::from(quant_limit)) as i16;
    (quantized + quant_limit) as u8
}

fn fast_scan_candidate_top_k_batch(
    query_count: usize,
    candidate_limit: usize,
) -> Vec<TurboQuantCandidateTopK> {
    (0..query_count)
        .map(|_| TurboQuantCandidateTopK::new(candidate_limit))
        .collect()
}

fn fast_scan_candidate_top_k_batch_with_limits(
    candidate_limits: &[usize],
) -> Vec<TurboQuantCandidateTopK> {
    candidate_limits
        .iter()
        .copied()
        .map(TurboQuantCandidateTopK::new)
        .collect()
}

fn merge_fast_scan_candidate_top_k_batch(
    mut lhs: Vec<TurboQuantCandidateTopK>,
    rhs: Vec<TurboQuantCandidateTopK>,
) -> Vec<TurboQuantCandidateTopK> {
    for (lhs_query, rhs_query) in lhs.iter_mut().zip(rhs) {
        for hit in rhs_query.into_hits() {
            lhs_query.push_distance(hit.key, hit.distance);
        }
    }
    lhs
}
