use std::array;

use rayon::prelude::*;
use selene_core::{TURBO_QUANT_BLOCK_ROWS, VectorTopK, VectorValue};
use wide::{i16x8, u8x16, u16x16};

use super::{TurboQuantVectorIndex, merge_candidate_top_k, query_component_for_score};

const FAST_SCAN_MAX_COMPONENTS: usize = (u16::MAX as usize) / 2;

impl TurboQuantVectorIndex {
    pub(super) fn slot_order_candidates_fast_scan(
        &self,
        rotated_query: &[f32],
        query_bias: f64,
        candidate_limit: usize,
    ) -> Option<VectorTopK<(usize, u32)>> {
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
    ) -> Vec<VectorTopK<(usize, u32)>> {
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

    fn slot_order_candidates_fast_scan_batch_parallel(
        &self,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
    ) -> Vec<VectorTopK<(usize, u32)>> {
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

    fn slot_order_candidates_fast_scan_batch_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        queries: &[PreparedFastScanQuery],
        candidate_limit: usize,
    ) -> Vec<VectorTopK<(usize, u32)>> {
        let mut candidates = fast_scan_candidate_top_k_batch(queries.len(), candidate_limit);
        let mut accumulators = vec![[u16x16::splat(0), u16x16::splat(0)]; queries.len()];
        let mut accumulator_lanes = vec![[[0_u16; 16], [0_u16; 16]]; queries.len()];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            accumulators.fill([u16x16::splat(0), u16x16::splat(0)]);
            for byte in 0..self.bytes_per_row {
                let codes = self.codes.block_byte(block, byte);
                let low_lanes = load_lanes(&codes[..16]);
                let high_lanes = load_lanes(&codes[16..]);
                for (accumulator, query) in accumulators.iter_mut().zip(queries) {
                    let byte_lut = &query.lut.bytes[byte];
                    accumulate_half(&mut accumulator[0], low_lanes, byte_lut);
                    accumulate_half(&mut accumulator[1], high_lanes, byte_lut);
                }
            }
            for (lanes, accumulator) in accumulator_lanes.iter_mut().zip(&accumulators) {
                *lanes = [accumulator[0].to_array(), accumulator[1].to_array()];
            }
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let slot = base_slot + lane;
                let entry = &self.entries[slot];
                if entry.deleted {
                    continue;
                }
                debug_assert_eq!(self.row_to_entry.get(&entry.row), Some(&slot));
                for ((candidate, query), lanes) in candidates
                    .iter_mut()
                    .zip(queries)
                    .zip(accumulator_lanes.iter())
                {
                    let centered = i32::from(lanes[lane / 16][lane % 16]) - query.lut.zero_sum;
                    let dot = query.query_bias + f64::from(centered) * query.lut.dequant;
                    let distance = -(dot * f64::from(self.row_scales[slot]));
                    candidate.push_distance((slot, entry.row), distance);
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
    ) -> VectorTopK<(usize, u32)> {
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
            .reduce(|| VectorTopK::new(candidate_limit), merge_candidate_top_k)
    }

    fn slot_order_candidates_fast_scan_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        lut: &FastScanQueryLut,
        query_bias: f64,
        candidate_limit: usize,
    ) -> VectorTopK<(usize, u32)> {
        let mut candidates = VectorTopK::new(candidate_limit);
        let mut accumulators = [u16x16::splat(0), u16x16::splat(0)];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            accumulators.fill(u16x16::splat(0));
            for (byte, byte_lut) in lut.bytes.iter().enumerate() {
                let codes = self.codes.block_byte(block, byte);
                accumulate_half(&mut accumulators[0], load_lanes(&codes[..16]), byte_lut);
                accumulate_half(&mut accumulators[1], load_lanes(&codes[16..]), byte_lut);
            }
            let lanes = [accumulators[0].to_array(), accumulators[1].to_array()];
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let slot = base_slot + lane;
                let entry = &self.entries[slot];
                if entry.deleted {
                    continue;
                }
                debug_assert_eq!(self.row_to_entry.get(&entry.row), Some(&slot));
                let centered = i32::from(lanes[lane / 16][lane % 16]) - lut.zero_sum;
                let dot = query_bias + f64::from(centered) * lut.dequant;
                let distance = -(dot * f64::from(self.row_scales[slot]));
                candidates.push_distance((slot, entry.row), distance);
            }
        }
        candidates
    }

    pub(super) fn fast_scan_lut(&self, rotated_query: &[f32]) -> Option<FastScanQueryLut> {
        let components = self.fast_scan_components()?;
        let quant_limit = ((usize::from(u16::MAX) / components.saturating_mul(2))
            .min(i8::MAX as usize)
            .max(1)) as i16;
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
        })
    }

    pub(super) fn supports_fast_scan_accumulator(&self) -> bool {
        self.fast_scan_components().is_some()
    }

    fn fast_scan_components(&self) -> Option<usize> {
        let components = self.bytes_per_row.checked_mul(2)?;
        (components <= FAST_SCAN_MAX_COMPONENTS).then_some(components)
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
) -> Vec<VectorTopK<(usize, u32)>> {
    (0..query_count)
        .map(|_| VectorTopK::new(candidate_limit))
        .collect()
}

fn merge_fast_scan_candidate_top_k_batch(
    mut lhs: Vec<VectorTopK<(usize, u32)>>,
    rhs: Vec<VectorTopK<(usize, u32)>>,
) -> Vec<VectorTopK<(usize, u32)>> {
    for (lhs_query, rhs_query) in lhs.iter_mut().zip(rhs) {
        for hit in rhs_query.into_hits() {
            lhs_query.push_distance(hit.key, hit.distance);
        }
    }
    lhs
}
