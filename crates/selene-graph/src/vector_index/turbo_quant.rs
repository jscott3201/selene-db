//! TurboQuant compressed candidate index for graph-side exact rerank.
//!
//! Durable state remains the vector-index registration plus primary graph
//! `VECTOR` properties. This derived index packs rotated unit-vector coordinates
//! into 4-bit codes and uses a byte LUT for candidate preselection. The graph
//! search layer reranks returned candidates against primary vectors with exact
//! cosine distance so the compressed index does not shadow full vector payloads.

use std::mem::size_of;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use selene_core::{
    CoreResult, MAX_VECTOR_DIMENSION, TURBO_QUANT_BLOCK_ROWS, TurboQuantBitWidth,
    TurboQuantBlockedCodes, TurboQuantCodebook, TurboQuantCodebookKind, VectorTopK, VectorValue,
};
use wide::f64x4;

use crate::error::{GraphError, GraphResult};
use crate::parallel_scan::should_parallelize_scan;

#[path = "turbo_quant/batch.rs"]
mod batch;
#[path = "turbo_quant/calibration.rs"]
mod calibration;
#[path = "turbo_quant/fast_scan.rs"]
mod fast_scan;
#[path = "turbo_quant/filter.rs"]
mod filter;

use calibration::quantile_calibration;

const TURBO_QUANT_BITS: u8 = 4;
const SLOT_ORDER_SCAN_STALE_RATIO: usize = 2;
const MIN_SLOT_ORDER_SCAN_ENTRIES: usize = 64;
#[cfg(not(test))]
const TURBO_QUANT_PARALLEL_MIN_ENTRIES: u64 = 4096;
#[cfg(test)]
const TURBO_QUANT_PARALLEL_MIN_ENTRIES: u64 = 8;
#[cfg(not(test))]
const TURBO_QUANT_PARALLEL_CHUNK_ENTRIES: usize = 1024;
#[cfg(test)]
const TURBO_QUANT_PARALLEL_CHUNK_ENTRIES: usize = 4;
#[cfg(not(test))]
const TURBO_QUANT_LOW_DIM_PARALLEL_CHUNK_ENTRIES: usize = 2048;
#[cfg(test)]
const TURBO_QUANT_LOW_DIM_PARALLEL_CHUNK_ENTRIES: usize = 4;
#[cfg(not(test))]
const TURBO_QUANT_FULL_LOW_DIM_PARALLEL_CHUNK_ENTRIES: usize = 4096;
#[cfg(test)]
const TURBO_QUANT_FULL_LOW_DIM_PARALLEL_CHUNK_ENTRIES: usize = 4;
const TURBO_QUANT_LOW_DIM_PARALLEL_MAX_DIMENSION: usize = 128;
const MIN_RECONSTRUCTED_INNER: f64 = 1e-10;

/// One approximate TurboQuant candidate over a graph row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TurboQuantVectorHit {
    pub(crate) row: u32,
    pub(crate) distance: f64,
}

type TurboQuantCandidateTopK = VectorTopK<u32>;

/// Estimated TurboQuant resident memory and structural counters.
///
/// Deletes and replacements compact slots immediately, so `deleted_entries`
/// reports invariant drift rather than expected rebuild pressure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TurboQuantMemoryUsage {
    pub(crate) entries: usize,
    pub(crate) live_entries: usize,
    pub(crate) deleted_entries: usize,
    pub(crate) code_bytes: usize,
    pub(crate) codebook_bytes: usize,
    pub(crate) calibration_bytes: usize,
    pub(crate) estimated_heap_bytes: usize,
    pub(crate) referenced_vector_bytes: usize,
}

/// Derived TurboQuant index for one cosine vector-index registration.
#[derive(Clone, Debug)]
pub(crate) struct TurboQuantVectorIndex {
    dimension: usize,
    bytes_per_row: usize,
    codebook: TurboQuantCodebook,
    codes: TurboQuantBlockedCodes,
    row_scales: Vec<f32>,
    shift: Vec<f32>,
    scale: Vec<f32>,
    inv_scale: Vec<f32>,
    rows: Vec<u32>,
    row_to_entry: FxHashMap<u32, u32>,
    live_entries: usize,
    collecting_bulk: bool,
    bulk_rotated: Vec<f32>,
}

impl TurboQuantVectorIndex {
    /// Construct an empty TurboQuant index for `dimension`.
    pub(crate) fn new(dimension: u32) -> GraphResult<Self> {
        let dimension = valid_dimension(dimension)?;
        let bit_width = TurboQuantBitWidth::new(TURBO_QUANT_BITS)
            .expect("production TurboQuant bit width is valid");
        let codebook =
            TurboQuantCodebook::new(TurboQuantCodebookKind::NormalLloydMax, bit_width, dimension)
                .map_err(codec_invariant)?;
        let codes =
            TurboQuantBlockedCodes::new(bit_width, dimension, 0).map_err(codec_invariant)?;
        Ok(Self {
            dimension,
            bytes_per_row: codes.bytes_per_row(),
            codebook,
            codes,
            row_scales: Vec::new(),
            shift: Vec::new(),
            scale: Vec::new(),
            inv_scale: Vec::new(),
            rows: Vec::new(),
            row_to_entry: FxHashMap::default(),
            live_entries: 0,
            collecting_bulk: true,
            bulk_rotated: Vec::new(),
        })
    }

    /// Insert or replace the current vector for a graph row.
    pub(crate) fn insert(&mut self, row: u32, vector: &VectorValue) -> GraphResult<()> {
        if let Some(slot_key) = self.row_to_entry.get(&row).copied() {
            return self.replace_slot(row, slot_index(slot_key), vector);
        }
        let slot = self.rows.len();
        let slot_key = slot_key(slot)?;
        if !self.collecting_bulk {
            self.codes.resize_rows(slot + 1).map_err(codec_invariant)?;
        }
        self.row_scales.push(1.0);
        let rotated = rotated_unit_vector(vector, self.dimension);
        if self.collecting_bulk {
            self.bulk_rotated.extend_from_slice(&rotated);
        }
        self.rows.push(row);
        self.row_to_entry.insert(row, slot_key);
        self.live_entries += 1;
        if !self.collecting_bulk {
            self.encode_slot(slot, &rotated)?;
        }
        Ok(())
    }

    /// Remove the current vector for `row`, if present.
    pub(crate) fn remove(&mut self, row: u32) {
        let Some(slot_key) = self.row_to_entry.remove(&row) else {
            return;
        };
        self.swap_remove_slot(slot_index(slot_key));
        self.live_entries = self.row_to_entry.len();
    }

    /// Recompute quantile calibration and packed codes after a bulk load.
    pub(crate) fn finish_bulk_load(&mut self) -> GraphResult<()> {
        if !self.collecting_bulk {
            return Ok(());
        }
        if self.live_entries == 0 {
            self.shift.clear();
            self.scale.clear();
            self.inv_scale.clear();
            self.bulk_rotated = Vec::new();
            self.collecting_bulk = false;
            return Ok(());
        }
        self.ensure_bulk_rotated_matches_slots()?;
        let rotated = std::mem::take(&mut self.bulk_rotated);
        let (shift, scale) = quantile_calibration(&rotated, self.dimension);
        self.inv_scale = scale.iter().map(|value| value.recip()).collect();
        self.shift = shift;
        self.scale = scale;
        if let Err(err) = self
            .codes
            .resize_rows(self.rows.len())
            .map_err(codec_invariant)
        {
            self.bulk_rotated = rotated;
            return Err(err);
        }
        for slot in 0..self.rows.len() {
            let start = slot * self.dimension;
            let end = start + self.dimension;
            if let Err(err) = self.encode_slot(slot, &rotated[start..end]) {
                self.bulk_rotated = rotated;
                return Err(err);
            }
        }
        self.collecting_bulk = false;
        Ok(())
    }

    /// Approximate candidate search over current row versions.
    pub(crate) fn candidates(
        &self,
        query: &VectorValue,
        k: usize,
        search_width: usize,
    ) -> CoreResult<Vec<TurboQuantVectorHit>> {
        if k == 0 || self.live_entries == 0 {
            return Ok(Vec::new());
        }
        let rotated_query = rotated_unit_vector(query, self.dimension);
        let query_bias = query_bias(&rotated_query, &self.shift);
        let candidate_limit = search_width.max(k).min(self.live_entries);
        let candidates = if self.should_scan_by_slot_order() {
            self.slot_order_candidates_fast_scan(&rotated_query, query_bias, candidate_limit)
                .unwrap_or_else(|| {
                    let byte_lut = self.byte_lut(&rotated_query);
                    self.slot_order_candidates(&byte_lut, query_bias, candidate_limit)
                })
        } else {
            let byte_lut = self.byte_lut(&rotated_query);
            self.live_map_candidates(&byte_lut, query_bias, candidate_limit)
        };

        Ok(candidates
            .into_hits()
            .into_iter()
            .map(|hit| TurboQuantVectorHit {
                row: hit.key,
                distance: hit.distance,
            })
            .collect())
    }

    fn should_scan_by_slot_order(&self) -> bool {
        self.rows.len() <= MIN_SLOT_ORDER_SCAN_ENTRIES
            || self.rows.len()
                <= self
                    .live_entries
                    .saturating_mul(SLOT_ORDER_SCAN_STALE_RATIO)
    }

    fn should_parallelize_slot_scan(&self, candidate_limit: usize) -> bool {
        should_parallelize_scan(
            self.rows.len() as u64,
            candidate_limit,
            TURBO_QUANT_PARALLEL_MIN_ENTRIES,
        )
    }

    fn parallel_chunk_blocks(&self) -> usize {
        let entries = if self.dimension <= TURBO_QUANT_LOW_DIM_PARALLEL_MAX_DIMENSION {
            TURBO_QUANT_LOW_DIM_PARALLEL_CHUNK_ENTRIES
        } else {
            TURBO_QUANT_PARALLEL_CHUNK_ENTRIES
        };
        entries.div_ceil(TURBO_QUANT_BLOCK_ROWS).max(1)
    }

    fn full_scan_parallel_chunk_blocks(&self) -> usize {
        let entries = if self.dimension <= TURBO_QUANT_LOW_DIM_PARALLEL_MAX_DIMENSION {
            TURBO_QUANT_FULL_LOW_DIM_PARALLEL_CHUNK_ENTRIES
        } else {
            TURBO_QUANT_PARALLEL_CHUNK_ENTRIES
        };
        entries.div_ceil(TURBO_QUANT_BLOCK_ROWS).max(1)
    }

    fn slot_order_candidates(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> TurboQuantCandidateTopK {
        if self.should_parallelize_slot_scan(candidate_limit) {
            return self.slot_order_candidates_parallel(byte_lut, query_bias, candidate_limit);
        }
        self.slot_order_candidates_blocks(
            0,
            self.codes.block_count(),
            byte_lut,
            query_bias,
            candidate_limit,
        )
    }

    fn slot_order_candidates_parallel(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> TurboQuantCandidateTopK {
        let chunk_blocks = self.full_scan_parallel_chunk_blocks();
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks)
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_blocks(start, end, byte_lut, query_bias, candidate_limit)
            })
            .reduce(
                || TurboQuantCandidateTopK::new(candidate_limit),
                merge_candidate_top_k,
            )
    }

    fn slot_order_candidates_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> TurboQuantCandidateTopK {
        let mut candidates = TurboQuantCandidateTopK::new(candidate_limit);
        let mut dots = [f64x4::ZERO; TURBO_QUANT_BLOCK_ROWS / 4];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
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
                    let Some(row) = self.live_row_at_slot(slot) else {
                        continue;
                    };
                    let distance = -(dot * f64::from(self.row_scales[slot]));
                    candidates.push_distance(row, distance);
                }
            }
        }
        candidates
    }

    fn live_map_candidates(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> TurboQuantCandidateTopK {
        let mut candidates = TurboQuantCandidateTopK::new(candidate_limit);
        for (&row, &slot_key) in &self.row_to_entry {
            let slot = slot_index(slot_key);
            let Some(stored_row) = self.rows.get(slot).copied() else {
                continue;
            };
            if stored_row != row {
                continue;
            }
            let distance = self.approx_distance_lut(slot, byte_lut, query_bias);
            candidates.push_distance(row, distance);
        }
        candidates
    }

    /// Return estimated TurboQuant memory usage.
    pub(crate) fn memory_usage(&self) -> TurboQuantMemoryUsage {
        let entries = self.rows.len();
        let deleted_entries = entries.saturating_sub(self.live_entries);
        let code_bytes = self.codes.estimated_bytes();
        let codebook_bytes = self.codebook.estimated_bytes();
        let calibration_bytes = self
            .shift
            .capacity()
            .saturating_add(self.scale.capacity())
            .saturating_add(self.inv_scale.capacity())
            .saturating_mul(size_of::<f32>());
        let bulk_rotated_bytes = self
            .bulk_rotated
            .capacity()
            .saturating_mul(size_of::<f32>());
        let estimated_heap_bytes = self
            .rows
            .capacity()
            .saturating_mul(size_of::<u32>())
            .saturating_add(
                self.row_to_entry
                    .capacity()
                    .saturating_mul(size_of::<(u32, u32)>()),
            )
            .saturating_add(self.row_scales.capacity().saturating_mul(size_of::<f32>()))
            .saturating_add(code_bytes)
            .saturating_add(codebook_bytes)
            .saturating_add(calibration_bytes)
            .saturating_add(bulk_rotated_bytes);
        TurboQuantMemoryUsage {
            entries,
            live_entries: self.live_entries,
            deleted_entries,
            code_bytes,
            codebook_bytes,
            calibration_bytes,
            estimated_heap_bytes,
            referenced_vector_bytes: 0,
        }
    }

    fn encode_slot(&mut self, slot: usize, rotated: &[f32]) -> GraphResult<()> {
        let mut reconstructed_inner = 0.0;
        for (dimension, value) in rotated.iter().copied().enumerate() {
            let calibrated = calibrate_value(value, dimension, &self.shift, &self.scale);
            let code = self
                .codebook
                .encode_scalar(calibrated)
                .map_err(codec_invariant)?;
            let reconstructed = reconstruct_value(
                usize::from(code),
                dimension,
                self.codebook.centroids(),
                &self.shift,
                &self.inv_scale,
            );
            reconstructed_inner += f64::from(value) * f64::from(reconstructed);
            self.codes
                .write(slot, dimension, code)
                .map_err(codec_invariant)?;
        }
        self.row_scales[slot] = (1.0 / reconstructed_inner.max(MIN_RECONSTRUCTED_INNER)) as f32;
        Ok(())
    }

    fn approx_distance_lut(&self, slot: usize, byte_lut: &[f64], query_bias: f64) -> f64 {
        let mut dot = query_bias;
        for byte in 0..self.bytes_per_row {
            let packed = usize::from(
                self.codes
                    .row_byte(slot, byte)
                    .expect("TurboQuant slot byte is in bounds"),
            );
            dot += byte_lut[byte * 256 + packed];
        }
        -(dot * f64::from(self.row_scales[slot]))
    }

    fn byte_lut(&self, rotated_query: &[f32]) -> Vec<f64> {
        let mut table = vec![0.0; self.bytes_per_row * 256];
        let centroids: [f64; 16] =
            std::array::from_fn(|code| f64::from(self.codebook.centroids()[code]));
        for byte in 0..self.bytes_per_row {
            let first_dim = byte * 2;
            let second_dim = first_dim + 1;
            let lut_base = byte * 256;
            if second_dim < self.dimension {
                let first_query = f64::from(query_component_for_score(
                    rotated_query[first_dim],
                    first_dim,
                    &self.inv_scale,
                ));
                let second_query = f64::from(query_component_for_score(
                    rotated_query[second_dim],
                    second_dim,
                    &self.inv_scale,
                ));
                for packed in 0..256 {
                    table[lut_base + packed] = first_query * centroids[packed & 0x0f]
                        + second_query * centroids[(packed >> 4) & 0x0f];
                }
            } else if first_dim < self.dimension {
                let first_query = f64::from(query_component_for_score(
                    rotated_query[first_dim],
                    first_dim,
                    &self.inv_scale,
                ));
                for packed in 0..256 {
                    table[lut_base + packed] = first_query * centroids[packed & 0x0f];
                }
            }
        }
        table
    }

    fn live_row_at_slot(&self, slot: usize) -> Option<u32> {
        let row = self.rows.get(slot).copied()?;
        debug_assert!(self.row_points_to_slot(row, slot));
        Some(row)
    }

    fn row_points_to_slot(&self, row: u32, slot: usize) -> bool {
        self.row_to_entry.get(&row).copied().map(slot_index) == Some(slot)
    }

    fn slot_for_row(&self, row: u32) -> Option<usize> {
        self.row_to_entry.get(&row).copied().map(slot_index)
    }

    fn ensure_bulk_rotated_matches_slots(&self) -> GraphResult<()> {
        let expected_len = self.rows.len().saturating_mul(self.dimension);
        if self.bulk_rotated.len() == expected_len {
            Ok(())
        } else {
            Err(GraphError::Inconsistent {
                reason: format!(
                    "TurboQuant bulk calibration has {} components for {} compact slots of dimension {}",
                    self.bulk_rotated.len(),
                    self.rows.len(),
                    self.dimension
                ),
            })
        }
    }

    fn replace_slot(&mut self, row: u32, slot: usize, vector: &VectorValue) -> GraphResult<()> {
        if self.rows.get(slot).copied() != Some(row) {
            return Err(GraphError::Inconsistent {
                reason: format!("TurboQuant row {row} points at invalid slot {slot}"),
            });
        }
        let rotated = rotated_unit_vector(vector, self.dimension);
        if self.collecting_bulk {
            return self.replace_bulk_rotated(slot, &rotated);
        }
        self.encode_slot(slot, &rotated)
    }

    fn replace_bulk_rotated(&mut self, slot: usize, rotated: &[f32]) -> GraphResult<()> {
        let start = slot * self.dimension;
        let end = start + self.dimension;
        let Some(pending) = self.bulk_rotated.get_mut(start..end) else {
            return Err(GraphError::Inconsistent {
                reason: format!("TurboQuant slot {slot} is missing bulk calibration data"),
            });
        };
        pending.copy_from_slice(rotated);
        Ok(())
    }

    fn swap_remove_slot(&mut self, slot: usize) {
        let last_slot = self.rows.len() - 1;
        let moved_row = (slot != last_slot).then(|| self.rows[last_slot]);
        if self.codes.rows() > 0 {
            self.codes
                .swap_remove_row(slot)
                .expect("TurboQuant live slot has encoded row bytes");
        }
        self.rows.swap_remove(slot);
        self.row_scales.swap_remove(slot);
        if self.collecting_bulk {
            self.swap_remove_bulk_rotated(slot, last_slot);
        }
        if let Some(row) = moved_row {
            let slot_key = slot_key(slot).expect("existing TurboQuant slot key remains valid");
            self.row_to_entry.insert(row, slot_key);
        }
    }

    fn swap_remove_bulk_rotated(&mut self, slot: usize, last_slot: usize) {
        if slot != last_slot {
            let source_start = last_slot * self.dimension;
            let destination_start = slot * self.dimension;
            for offset in 0..self.dimension {
                self.bulk_rotated[destination_start + offset] =
                    self.bulk_rotated[source_start + offset];
            }
        }
        self.bulk_rotated.truncate(last_slot * self.dimension);
    }
}

fn merge_candidate_top_k(
    mut lhs: TurboQuantCandidateTopK,
    rhs: TurboQuantCandidateTopK,
) -> TurboQuantCandidateTopK {
    for hit in rhs.into_hits() {
        lhs.push_distance(hit.key, hit.distance);
    }
    lhs
}

fn valid_dimension(dimension: u32) -> GraphResult<usize> {
    let dimension_usize = usize::try_from(dimension)
        .map_err(|_| GraphError::VectorIndexInvalidDimension { dimension })?;
    if dimension_usize == 0 || dimension_usize > MAX_VECTOR_DIMENSION {
        Err(GraphError::VectorIndexInvalidDimension { dimension })
    } else {
        Ok(dimension_usize)
    }
}

fn codec_invariant(err: selene_core::TurboQuantCodecError) -> GraphError {
    GraphError::Inconsistent {
        reason: format!("TurboQuant index invariant failed: {err}"),
    }
}

fn slot_key(slot: usize) -> GraphResult<u32> {
    u32::try_from(slot).map_err(|_| GraphError::Inconsistent {
        reason: "TurboQuant slot index exceeds u32::MAX".to_owned(),
    })
}

fn slot_index(slot: u32) -> usize {
    usize::try_from(slot).expect("TurboQuant slot key always fits usize")
}

fn calibrate_value(value: f32, dim: usize, shift: &[f32], scale: &[f32]) -> f32 {
    if shift.is_empty() {
        value
    } else {
        (value + shift[dim]) * scale[dim]
    }
}

fn reconstruct_value(
    code: usize,
    dim: usize,
    centroids: &[f32],
    shift: &[f32],
    inv: &[f32],
) -> f32 {
    if shift.is_empty() {
        centroids[code]
    } else {
        centroids[code] * inv[dim] - shift[dim]
    }
}

fn query_component_for_score(value: f32, dim: usize, inv_scale: &[f32]) -> f32 {
    if inv_scale.is_empty() {
        value
    } else {
        value * inv_scale[dim]
    }
}

fn query_bias(rotated_query: &[f32], shift: &[f32]) -> f64 {
    if shift.is_empty() {
        return 0.0;
    }
    -rotated_query
        .iter()
        .zip(shift)
        .map(|(query, shift)| f64::from(*query) * f64::from(*shift))
        .sum::<f64>()
}

fn rotated_unit_vector(vector: &VectorValue, dimension: usize) -> Vec<f32> {
    debug_assert_eq!(vector.dimension(), dimension);
    let mut output = vec![0.0; dimension];
    let length_squared = vector
        .as_slice()
        .iter()
        .map(|value| *value * *value)
        .sum::<f32>();
    if length_squared == 0.0 {
        return output;
    }
    let inverse_length = length_squared.sqrt().recip();
    for (dim, value) in vector.as_slice().iter().enumerate() {
        output[dim] = *value * inverse_length * random_sign(dim);
    }
    block_hadamard_transform(&mut output);
    output
}

fn block_hadamard_transform(values: &mut [f32]) {
    let mut offset = 0;
    while offset < values.len() {
        let block_len = largest_power_of_two_at_most(values.len() - offset);
        let block = &mut values[offset..offset + block_len];
        hadamard_transform(block);
        let scale = (block_len as f32).sqrt().recip();
        for value in block {
            *value *= scale;
        }
        offset += block_len;
    }
}

fn largest_power_of_two_at_most(value: usize) -> usize {
    1_usize << (usize::BITS - 1 - value.leading_zeros())
}

fn hadamard_transform(values: &mut [f32]) {
    let mut span = 1;
    while span < values.len() {
        for block in (0..values.len()).step_by(span * 2) {
            for dim in block..block + span {
                let left = values[dim];
                let right = values[dim + span];
                values[dim] = left + right;
                values[dim + span] = left - right;
            }
        }
        span *= 2;
    }
}

fn random_sign(dim: usize) -> f32 {
    if splitmix64(dim as u64 ^ 0x9e37_79b9_7f4a_7c15) & 1 == 0 {
        1.0
    } else {
        -1.0
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
#[path = "turbo_quant/tests.rs"]
mod tests;
