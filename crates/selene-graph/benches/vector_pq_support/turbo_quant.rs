use std::mem::size_of;

use selene_core::{
    TURBO_QUANT_BLOCK_ROWS, TurboQuantBitWidth as CoreTurboQuantBitWidth,
    TurboQuantBlockedCodes as CoreTurboQuantBlockedCodes,
    TurboQuantCodebook as CoreTurboQuantCodebook,
    TurboQuantPackedCodes as CoreTurboQuantPackedCodes, VectorMetric, VectorTopK, VectorValue,
    exact_vector_top_k,
};
use wide::f64x4;

#[path = "turbo_quant/fast_scan.rs"]
mod fast_scan;

#[derive(Clone, Copy, Debug)]
pub(crate) enum TurboQuantCodebook {
    ClippedUniform,
    NormalLloydMax,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TurboQuantCalibration {
    None,
    Quantile,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TurboQuantScorer {
    Scalar,
    ByteLut,
    BlockedByteLut,
    BlockedWideByteLut,
    BlockedFastScanLut,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TurboQuantVariant {
    pub(crate) name: &'static str,
    pub(crate) bit_width: usize,
    pub(crate) candidates: usize,
    pub(crate) codebook: TurboQuantCodebook,
    pub(crate) calibration: TurboQuantCalibration,
    pub(crate) scorer: TurboQuantScorer,
}

#[derive(Debug)]
pub(crate) struct TurboQuantIndex {
    variant: TurboQuantVariant,
    dimension: usize,
    bytes_per_vector: usize,
    codebook: CoreTurboQuantCodebook,
    shift: Vec<f32>,
    inv_scale: Vec<f32>,
    scales: Vec<f32>,
    codes: CoreTurboQuantPackedCodes,
    blocked_codes: Option<CoreTurboQuantBlockedCodes>,
}

impl TurboQuantIndex {
    pub(crate) fn build(vectors: &[VectorValue], variant: TurboQuantVariant) -> Self {
        assert!(matches!(variant.bit_width, 2..=4));
        let bit_width = CoreTurboQuantBitWidth::new(variant.bit_width as u8)
            .expect("TurboQuant benchmark variants use supported bit widths");
        let dimension = vectors
            .first()
            .map(VectorValue::dimension)
            .expect("TurboQuant benchmark requires at least one vector");
        assert_eq!(dimension % u8::BITS as usize, 0);
        assert!(vectors.iter().all(|vector| vector.dimension() == dimension));
        let bytes_per_vector = dimension * variant.bit_width / u8::BITS as usize;
        let codebook = match variant.codebook {
            TurboQuantCodebook::ClippedUniform => {
                CoreTurboQuantCodebook::clipped_uniform(bit_width, dimension)
            }
            TurboQuantCodebook::NormalLloydMax => {
                CoreTurboQuantCodebook::normal_lloyd_max(bit_width, dimension)
            }
        };
        let codebook =
            codebook.expect("TurboQuant benchmark vectors use valid non-zero dimensions");
        let rotated_vectors = rotated_vectors(vectors, dimension);
        let (shift, scale) = match variant.calibration {
            TurboQuantCalibration::None => (Vec::new(), Vec::new()),
            TurboQuantCalibration::Quantile => quantile_calibration(&rotated_vectors, dimension),
        };
        let inv_scale = scale.iter().map(|value| value.recip()).collect::<Vec<_>>();
        let mut scales = Vec::with_capacity(vectors.len());
        let mut codes = CoreTurboQuantPackedCodes::new(bit_width, dimension, vectors.len())
            .expect("TurboQuant benchmark vectors use packable dimensions");
        assert_eq!(bytes_per_vector, codes.bytes_per_row());
        for (row, rotated) in rotated_vectors.chunks_exact(dimension).enumerate() {
            let mut reconstructed_inner = 0.0;
            for (dim, value) in rotated.iter().enumerate() {
                let calibrated = calibrate_value(*value, dim, &shift, &scale);
                let code = codebook
                    .encode_scalar(calibrated)
                    .expect("rotated benchmark vectors are finite");
                let reconstructed = reconstruct_value(
                    usize::from(code),
                    dim,
                    codebook.centroids(),
                    &shift,
                    &inv_scale,
                );
                reconstructed_inner += f64::from(*value) * f64::from(reconstructed);
                codes
                    .write(row, dim, code)
                    .expect("TurboQuant benchmark writes in-bounds codes");
            }
            scales.push((1.0 / reconstructed_inner.max(1e-10)) as f32);
        }
        let blocked_codes = matches!(
            variant.scorer,
            TurboQuantScorer::BlockedByteLut
                | TurboQuantScorer::BlockedWideByteLut
                | TurboQuantScorer::BlockedFastScanLut
        )
        .then(|| {
            CoreTurboQuantBlockedCodes::from_row_major(&codes)
                .expect("TurboQuant benchmark row-major codes repack into blocks")
        });
        Self {
            variant,
            dimension,
            bytes_per_vector,
            codebook,
            shift,
            inv_scale,
            scales,
            codes,
            blocked_codes,
        }
    }

    pub(crate) fn search_all(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        k: usize,
    ) -> Vec<usize> {
        if self.uses_blocked_lut() {
            return self.search_all_blocked(vectors, query, k);
        }
        if self.uses_blocked_wide_lut() {
            return self.search_all_blocked_wide(vectors, query, k);
        }
        if self.uses_blocked_fast_scan_lut() {
            return self.search_all_blocked_fast_scan(vectors, query, k);
        }
        self.search_rows(vectors, query, 0..vectors.len(), k)
    }

    pub(crate) fn search_rows(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        rows: impl IntoIterator<Item = usize>,
        k: usize,
    ) -> Vec<usize> {
        assert_eq!(query.dimension(), self.dimension);
        let mut rotated_query = vec![0.0; self.dimension];
        let _query_length = rotate_unit_vector(query, &mut rotated_query);
        let query_bias = query_bias(&rotated_query, &self.shift);
        let byte_lut = self.uses_byte_lut().then(|| self.byte_lut(&rotated_query));
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in rows {
            let distance = match byte_lut.as_deref() {
                Some(lut) => self.approx_distance_lut(row, lut, query_bias),
                None => self.approx_distance_scalar(row, &rotated_query, query_bias),
            };
            candidates.push_distance(row, distance);
        }
        let candidate_ids = candidates
            .into_hits()
            .into_iter()
            .map(|hit| hit.key)
            .collect::<Vec<_>>();
        let hits = exact_vector_top_k(
            VectorMetric::Cosine,
            query,
            candidate_ids.iter().map(|&row| (row, &vectors[row])),
            k,
        )
        .expect("TurboQuant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        // Blocked bench rows report the intended replacement layout size. The
        // benchmark still keeps row-major codes around for construction parity.
        let code_bytes = self.blocked_codes.as_ref().map_or_else(
            || self.codes.estimated_bytes(),
            CoreTurboQuantBlockedCodes::estimated_bytes,
        );
        code_bytes.saturating_add(
            self.scales
                .len()
                .saturating_add(self.codebook.centroids().len())
                .saturating_add(self.shift.len())
                .saturating_add(self.inv_scale.len())
                .saturating_mul(size_of::<f32>()),
        )
    }

    fn uses_byte_lut(&self) -> bool {
        matches!(
            self.variant.scorer,
            TurboQuantScorer::ByteLut | TurboQuantScorer::BlockedByteLut
        )
    }

    fn uses_blocked_lut(&self) -> bool {
        matches!(self.variant.scorer, TurboQuantScorer::BlockedByteLut)
    }

    fn uses_blocked_wide_lut(&self) -> bool {
        matches!(self.variant.scorer, TurboQuantScorer::BlockedWideByteLut)
    }

    fn uses_blocked_fast_scan_lut(&self) -> bool {
        matches!(self.variant.scorer, TurboQuantScorer::BlockedFastScanLut)
    }

    fn search_all_blocked(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        k: usize,
    ) -> Vec<usize> {
        assert_eq!(query.dimension(), self.dimension);
        let mut rotated_query = vec![0.0; self.dimension];
        let _query_length = rotate_unit_vector(query, &mut rotated_query);
        let query_bias = query_bias(&rotated_query, &self.shift);
        let byte_lut = self.byte_lut(&rotated_query);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        let blocked = self
            .blocked_codes
            .as_ref()
            .expect("blocked scorer builds blocked code storage");
        let mut dots = [0.0; TURBO_QUANT_BLOCK_ROWS];
        for block in 0..blocked.block_count() {
            let block_len = blocked.block_len(block);
            dots[..block_len].fill(query_bias);
            for byte in 0..self.bytes_per_vector {
                let lut_base = byte * 256;
                let codes = blocked.block_byte(block, byte);
                for lane in 0..block_len {
                    dots[lane] += byte_lut[lut_base + usize::from(codes[lane])];
                }
            }
            let base_row = block * TURBO_QUANT_BLOCK_ROWS;
            for (lane, dot) in dots[..block_len].iter().copied().enumerate() {
                let row = base_row + lane;
                candidates.push_distance(row, -(dot * f64::from(self.scales[row])));
            }
        }
        let candidate_ids = candidates
            .into_hits()
            .into_iter()
            .map(|hit| hit.key)
            .collect::<Vec<_>>();
        let hits = exact_vector_top_k(
            VectorMetric::Cosine,
            query,
            candidate_ids.iter().map(|&row| (row, &vectors[row])),
            k,
        )
        .expect("TurboQuant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    fn search_all_blocked_wide(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        k: usize,
    ) -> Vec<usize> {
        assert_eq!(query.dimension(), self.dimension);
        let mut rotated_query = vec![0.0; self.dimension];
        let _query_length = rotate_unit_vector(query, &mut rotated_query);
        let query_bias = query_bias(&rotated_query, &self.shift);
        let byte_lut = self.byte_lut(&rotated_query);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        let blocked = self
            .blocked_codes
            .as_ref()
            .expect("blocked wide scorer builds blocked code storage");
        let mut dots = [f64x4::ZERO; TURBO_QUANT_BLOCK_ROWS / 4];
        for block in 0..blocked.block_count() {
            let block_len = blocked.block_len(block);
            dots.fill(f64x4::splat(query_bias));
            for byte in 0..self.bytes_per_vector {
                let lut_base = byte * 256;
                let codes = blocked.block_byte(block, byte);
                for lane_base in (0..TURBO_QUANT_BLOCK_ROWS).step_by(4) {
                    dots[lane_base / 4] += f64x4::from([
                        byte_lut[lut_base + usize::from(codes[lane_base])],
                        byte_lut[lut_base + usize::from(codes[lane_base + 1])],
                        byte_lut[lut_base + usize::from(codes[lane_base + 2])],
                        byte_lut[lut_base + usize::from(codes[lane_base + 3])],
                    ]);
                }
            }
            let base_row = block * TURBO_QUANT_BLOCK_ROWS;
            for lane_base in (0..block_len).step_by(4) {
                let dot_lanes: [f64; 4] = dots[lane_base / 4].into();
                let active_lanes = (block_len - lane_base).min(4);
                for (lane_offset, dot) in dot_lanes.into_iter().take(active_lanes).enumerate() {
                    let row = base_row + lane_base + lane_offset;
                    candidates.push_distance(row, -(dot * f64::from(self.scales[row])));
                }
            }
        }
        let candidate_ids = candidates
            .into_hits()
            .into_iter()
            .map(|hit| hit.key)
            .collect::<Vec<_>>();
        let hits = exact_vector_top_k(
            VectorMetric::Cosine,
            query,
            candidate_ids.iter().map(|&row| (row, &vectors[row])),
            k,
        )
        .expect("TurboQuant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    fn approx_distance_scalar(&self, row: usize, rotated_query: &[f32], query_bias: f64) -> f64 {
        let mut dot = query_bias;
        for (dim, query_component) in rotated_query.iter().enumerate() {
            let code = usize::from(
                self.codes
                    .read(row, dim)
                    .expect("TurboQuant benchmark reads in-bounds codes"),
            );
            let component = query_component_for_score(*query_component, dim, &self.inv_scale);
            dot += f64::from(component) * f64::from(self.codebook.centroids()[code]);
        }
        -(dot * f64::from(self.scales[row]))
    }

    fn approx_distance_lut(&self, row: usize, byte_lut: &[f64], query_bias: f64) -> f64 {
        let code_offset = row * self.bytes_per_vector;
        let codes = self.codes.as_bytes();
        let mut dot = query_bias;
        for byte in 0..self.bytes_per_vector {
            let packed = usize::from(codes[code_offset + byte]);
            dot += byte_lut[byte * 256 + packed];
        }
        -(dot * f64::from(self.scales[row]))
    }

    fn byte_lut(&self, rotated_query: &[f32]) -> Vec<f64> {
        assert_eq!(self.variant.bit_width, 4);
        let mut table = vec![0.0; self.bytes_per_vector * 256];
        for byte in 0..self.bytes_per_vector {
            let first_dim = byte * 2;
            let first_query =
                query_component_for_score(rotated_query[first_dim], first_dim, &self.inv_scale);
            let second_query = query_component_for_score(
                rotated_query[first_dim + 1],
                first_dim + 1,
                &self.inv_scale,
            );
            for packed in 0..256 {
                let first_code = packed & 0x0f;
                let second_code = (packed >> 4) & 0x0f;
                table[byte * 256 + packed] = f64::from(first_query)
                    * f64::from(self.codebook.centroids()[first_code])
                    + f64::from(second_query) * f64::from(self.codebook.centroids()[second_code]);
            }
        }
        table
    }
}

fn rotated_vectors(vectors: &[VectorValue], dimension: usize) -> Vec<f32> {
    let mut rotated_vectors = vec![0.0; vectors.len() * dimension];
    for (row, vector) in vectors.iter().enumerate() {
        rotate_unit_vector(
            vector,
            &mut rotated_vectors[row * dimension..(row + 1) * dimension],
        );
    }
    rotated_vectors
}

fn quantile_calibration(rotated_vectors: &[f32], dimension: usize) -> (Vec<f32>, Vec<f32>) {
    let rows = rotated_vectors.len() / dimension;
    let target_low = -1.644_853_6_f32 / (dimension as f32).sqrt();
    let target_high = -target_low;
    let target_span = target_high - target_low;
    let low_index = ((rows as f64) * 0.05) as usize;
    let high_index = (((rows as f64) * 0.95) as usize).min(rows.saturating_sub(1));
    let mut shift = vec![0.0; dimension];
    let mut scale = vec![1.0; dimension];
    let mut coordinate = vec![0.0; rows];

    for dim in 0..dimension {
        for row in 0..rows {
            coordinate[row] = rotated_vectors[row * dimension + dim];
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
    codebook: &[f32],
    shift: &[f32],
    inv_scale: &[f32],
) -> f32 {
    if shift.is_empty() {
        codebook[code]
    } else {
        codebook[code] * inv_scale[dim] - shift[dim]
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

fn rotate_unit_vector(vector: &VectorValue, output: &mut [f32]) -> f32 {
    assert_eq!(vector.dimension(), output.len());
    let length_squared = vector
        .as_slice()
        .iter()
        .map(|value| *value * *value)
        .sum::<f32>();
    if length_squared == 0.0 {
        output.fill(0.0);
        return 0.0;
    }

    let length = length_squared.sqrt();
    for (dim, value) in vector.as_slice().iter().enumerate() {
        output[dim] = *value / length * random_sign(dim);
    }
    block_hadamard_transform(output);
    length
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
