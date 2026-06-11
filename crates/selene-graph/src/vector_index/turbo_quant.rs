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

use crate::error::{GraphError, GraphResult};
use crate::parallel_scan::should_parallelize_scan;

#[path = "turbo_quant/batch.rs"]
mod batch;

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
const MIN_RECONSTRUCTED_INNER: f64 = 1e-10;
const QUANTILE_LOW_Z: f32 = -1.644_853_6;

/// One approximate TurboQuant candidate over a graph row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TurboQuantVectorHit {
    pub(crate) row: u32,
    pub(crate) distance: f64,
}

/// Estimated TurboQuant resident memory and structural counters.
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
    entries: Vec<TurboQuantEntry>,
    row_to_entry: FxHashMap<u32, usize>,
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
            entries: Vec::new(),
            row_to_entry: FxHashMap::default(),
            live_entries: 0,
            collecting_bulk: true,
            bulk_rotated: Vec::new(),
        })
    }

    /// Insert or replace the current vector for a graph row.
    pub(crate) fn insert(&mut self, row: u32, vector: &VectorValue) -> GraphResult<()> {
        self.remove(row);
        let slot = self.entries.len();
        self.codes.resize_rows(slot + 1).map_err(codec_invariant)?;
        self.row_scales.push(1.0);
        let rotated = rotated_unit_vector(vector, self.dimension);
        if self.collecting_bulk {
            self.bulk_rotated.extend_from_slice(&rotated);
        }
        self.entries.push(TurboQuantEntry {
            row,
            deleted: false,
        });
        self.row_to_entry.insert(row, slot);
        self.live_entries += 1;
        self.encode_slot(slot, &rotated)?;
        Ok(())
    }

    /// Mark the current vector for `row` stale, if present.
    pub(crate) fn remove(&mut self, row: u32) {
        let Some(slot) = self.row_to_entry.remove(&row) else {
            return;
        };
        if let Some(entry) = self.entries.get_mut(slot)
            && !entry.deleted
        {
            entry.deleted = true;
            self.live_entries = self.live_entries.saturating_sub(1);
        }
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
        let live_slots = self.live_entry_slots();
        let rotated = self.rotated_live_vectors(&live_slots)?;
        let (shift, scale) = quantile_calibration(&rotated, self.dimension);
        self.inv_scale = scale.iter().map(|value| value.recip()).collect();
        self.shift = shift;
        self.scale = scale;
        for (offset, slot) in live_slots.iter().copied().enumerate() {
            let start = offset * self.dimension;
            let end = start + self.dimension;
            self.encode_slot(slot, &rotated[start..end])?;
        }
        self.bulk_rotated = Vec::new();
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
        let byte_lut = self.byte_lut(&rotated_query);
        let candidate_limit = search_width.max(k).min(self.live_entries);
        let candidates = if self.should_scan_by_slot_order() {
            self.slot_order_candidates(&byte_lut, query_bias, candidate_limit)
        } else {
            self.live_map_candidates(&byte_lut, query_bias, candidate_limit)
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

    fn should_scan_by_slot_order(&self) -> bool {
        self.entries.len() <= MIN_SLOT_ORDER_SCAN_ENTRIES
            || self.entries.len()
                <= self
                    .live_entries
                    .saturating_mul(SLOT_ORDER_SCAN_STALE_RATIO)
    }

    fn should_parallelize_slot_scan(&self, candidate_limit: usize) -> bool {
        should_parallelize_scan(
            self.entries.len() as u64,
            candidate_limit,
            TURBO_QUANT_PARALLEL_MIN_ENTRIES,
        )
    }

    fn slot_order_candidates(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> VectorTopK<(usize, u32)> {
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
    ) -> VectorTopK<(usize, u32)> {
        let chunk_blocks = TURBO_QUANT_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_blocks(start, end, byte_lut, query_bias, candidate_limit)
            })
            .reduce(|| VectorTopK::new(candidate_limit), merge_candidate_top_k)
    }

    fn slot_order_candidates_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> VectorTopK<(usize, u32)> {
        let mut candidates = VectorTopK::new(candidate_limit);
        let mut dots = [0.0; TURBO_QUANT_BLOCK_ROWS];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            dots[..block_len].fill(query_bias);
            for byte in 0..self.bytes_per_row {
                let lut_base = byte * 256;
                let codes = self.codes.block_byte(block, byte);
                for lane in 0..block_len {
                    dots[lane] += byte_lut[lut_base + usize::from(codes[lane])];
                }
            }
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for (lane, dot) in dots[..block_len].iter().copied().enumerate() {
                let slot = base_slot + lane;
                let entry = &self.entries[slot];
                if entry.deleted {
                    continue;
                }
                debug_assert_eq!(self.row_to_entry.get(&entry.row), Some(&slot));
                let distance = -(dot * f64::from(self.row_scales[slot]));
                candidates.push_distance((slot, entry.row), distance);
            }
        }
        candidates
    }

    fn live_map_candidates(
        &self,
        byte_lut: &[f64],
        query_bias: f64,
        candidate_limit: usize,
    ) -> VectorTopK<(usize, u32)> {
        let mut candidates = VectorTopK::new(candidate_limit);
        for (&row, &slot) in &self.row_to_entry {
            let Some(entry) = self.entries.get(slot) else {
                continue;
            };
            if entry.deleted || entry.row != row {
                continue;
            }
            let distance = self.approx_distance_lut(slot, byte_lut, query_bias);
            candidates.push_distance((slot, row), distance);
        }
        candidates
    }

    /// Return estimated TurboQuant memory usage.
    pub(crate) fn memory_usage(&self) -> TurboQuantMemoryUsage {
        let entries = self.entries.len();
        let deleted_entries = self.entries.iter().filter(|entry| entry.deleted).count();
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
            .entries
            .capacity()
            .saturating_mul(size_of::<TurboQuantEntry>())
            .saturating_add(
                self.row_to_entry
                    .capacity()
                    .saturating_mul(size_of::<(u32, usize)>()),
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
        for byte in 0..self.bytes_per_row {
            let first_dim = byte * 2;
            let second_dim = first_dim + 1;
            for packed in 0..256 {
                let first = (first_dim < self.dimension).then(|| {
                    let query = query_component_for_score(
                        rotated_query[first_dim],
                        first_dim,
                        &self.inv_scale,
                    );
                    f64::from(query) * f64::from(self.codebook.centroids()[packed & 0x0f])
                });
                let second = (second_dim < self.dimension).then(|| {
                    let query = query_component_for_score(
                        rotated_query[second_dim],
                        second_dim,
                        &self.inv_scale,
                    );
                    f64::from(query) * f64::from(self.codebook.centroids()[(packed >> 4) & 0x0f])
                });
                table[byte * 256 + packed] = first.unwrap_or_default() + second.unwrap_or_default();
            }
        }
        table
    }

    fn live_entry_slots(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(slot, entry)| {
                (!entry.deleted && self.row_to_entry.get(&entry.row) == Some(&slot)).then_some(slot)
            })
            .collect()
    }

    fn rotated_live_vectors(&self, live_slots: &[usize]) -> GraphResult<Vec<f32>> {
        let mut rotated = Vec::with_capacity(live_slots.len() * self.dimension);
        for &slot in live_slots {
            let start = slot * self.dimension;
            let end = start + self.dimension;
            let Some(pending) = self.bulk_rotated.get(start..end) else {
                return Err(GraphError::Inconsistent {
                    reason: format!("TurboQuant live slot {slot} is missing bulk calibration data"),
                });
            };
            rotated.extend_from_slice(pending);
        }
        Ok(rotated)
    }
}

#[derive(Clone, Debug)]
struct TurboQuantEntry {
    row: u32,
    deleted: bool,
}

fn merge_candidate_top_k(
    mut lhs: VectorTopK<(usize, u32)>,
    rhs: VectorTopK<(usize, u32)>,
) -> VectorTopK<(usize, u32)> {
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

fn quantile_calibration(rotated: &[f32], dimension: usize) -> (Vec<f32>, Vec<f32>) {
    let rows = rotated.len() / dimension;
    let target_low = QUANTILE_LOW_Z / (dimension as f32).sqrt();
    let target_high = -target_low;
    let target_span = target_high - target_low;
    let low_index = ((rows as f64) * 0.05) as usize;
    let high_index = (((rows as f64) * 0.95) as usize).min(rows.saturating_sub(1));
    let mut shift = vec![0.0; dimension];
    let mut scale = vec![1.0; dimension];
    let mut coordinate = vec![0.0; rows];

    for dim in 0..dimension {
        for row in 0..rows {
            coordinate[row] = rotated[row * dimension + dim];
        }
        coordinate.sort_unstable_by(f32::total_cmp);
        let source_low = coordinate[low_index];
        let source_high = coordinate[high_index];
        let source_span = source_high - source_low;
        if source_span > 1e-6 {
            scale[dim] = target_span / source_span;
            shift[dim] = target_low / scale[dim] - source_low;
        }
    }
    (shift, scale)
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
