use std::mem::size_of;

use selene_core::{VectorMetric, VectorTopK, VectorValue, exact_vector_top_k};

use super::DIMENSION;

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
    bytes_per_vector: usize,
    codebook: Vec<f32>,
    shift: Vec<f32>,
    inv_scale: Vec<f32>,
    scales: Vec<f32>,
    codes: Vec<u8>,
}

impl TurboQuantIndex {
    pub(crate) fn build(vectors: &[VectorValue], variant: TurboQuantVariant) -> Self {
        assert!(matches!(variant.bit_width, 2..=4));
        assert!(DIMENSION.is_power_of_two());
        let bytes_per_vector = DIMENSION * variant.bit_width / u8::BITS as usize;
        let codebook = match variant.codebook {
            TurboQuantCodebook::ClippedUniform => clipped_uniform_codebook(variant.bit_width),
            TurboQuantCodebook::NormalLloydMax => normal_lloyd_codebook(variant.bit_width),
        };
        let rotated_vectors = rotated_vectors(vectors);
        let (shift, scale) = match variant.calibration {
            TurboQuantCalibration::None => (Vec::new(), Vec::new()),
            TurboQuantCalibration::Quantile => quantile_calibration(&rotated_vectors),
        };
        let inv_scale = scale.iter().map(|value| value.recip()).collect::<Vec<_>>();
        let mut scales = Vec::with_capacity(vectors.len());
        let mut codes = vec![0; vectors.len() * bytes_per_vector];
        for (row, rotated) in rotated_vectors.chunks_exact(DIMENSION).enumerate() {
            let mut reconstructed_inner = 0.0;
            for (dim, value) in rotated.iter().enumerate() {
                let calibrated = calibrate_value(*value, dim, &shift, &scale);
                let code = nearest_code(calibrated, &codebook);
                let reconstructed = reconstruct_value(code, dim, &codebook, &shift, &inv_scale);
                reconstructed_inner += f64::from(*value) * f64::from(reconstructed);
                write_code(
                    &mut codes,
                    row,
                    bytes_per_vector,
                    variant.bit_width,
                    dim,
                    code,
                );
            }
            scales.push((1.0 / reconstructed_inner.max(1e-10)) as f32);
        }
        Self {
            variant,
            bytes_per_vector,
            codebook,
            shift,
            inv_scale,
            scales,
            codes,
        }
    }

    pub(crate) fn search_all(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        k: usize,
    ) -> Vec<usize> {
        self.search_rows(vectors, query, 0..vectors.len(), k)
    }

    pub(crate) fn search_rows(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        rows: impl IntoIterator<Item = usize>,
        k: usize,
    ) -> Vec<usize> {
        let mut rotated_query = vec![0.0; DIMENSION];
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
        self.codes.len().saturating_add(
            self.scales
                .len()
                .saturating_add(self.codebook.len())
                .saturating_add(self.shift.len())
                .saturating_add(self.inv_scale.len())
                .saturating_mul(size_of::<f32>()),
        )
    }

    fn uses_byte_lut(&self) -> bool {
        matches!(self.variant.scorer, TurboQuantScorer::ByteLut)
    }

    fn approx_distance_scalar(&self, row: usize, rotated_query: &[f32], query_bias: f64) -> f64 {
        let mut dot = query_bias;
        for (dim, query_component) in rotated_query.iter().enumerate() {
            let code = read_code(
                &self.codes,
                row,
                self.bytes_per_vector,
                self.variant.bit_width,
                dim,
            );
            let component = query_component_for_score(*query_component, dim, &self.inv_scale);
            dot += f64::from(component) * f64::from(self.codebook[code]);
        }
        -(dot * f64::from(self.scales[row]))
    }

    fn approx_distance_lut(&self, row: usize, byte_lut: &[f64], query_bias: f64) -> f64 {
        let code_offset = row * self.bytes_per_vector;
        let mut dot = query_bias;
        for byte in 0..self.bytes_per_vector {
            let packed = usize::from(self.codes[code_offset + byte]);
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
                    * f64::from(self.codebook[first_code])
                    + f64::from(second_query) * f64::from(self.codebook[second_code]);
            }
        }
        table
    }
}

fn rotated_vectors(vectors: &[VectorValue]) -> Vec<f32> {
    let mut rotated_vectors = vec![0.0; vectors.len() * DIMENSION];
    for (row, vector) in vectors.iter().enumerate() {
        rotate_unit_vector(
            vector,
            &mut rotated_vectors[row * DIMENSION..(row + 1) * DIMENSION],
        );
    }
    rotated_vectors
}

fn quantile_calibration(rotated_vectors: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let rows = rotated_vectors.len() / DIMENSION;
    let target_low = -1.644_853_6_f32 / (DIMENSION as f32).sqrt();
    let target_high = -target_low;
    let target_span = target_high - target_low;
    let low_index = ((rows as f64) * 0.05) as usize;
    let high_index = (((rows as f64) * 0.95) as usize).min(rows.saturating_sub(1));
    let mut shift = vec![0.0; DIMENSION];
    let mut scale = vec![1.0; DIMENSION];
    let mut coordinate = vec![0.0; rows];

    for dim in 0..DIMENSION {
        for row in 0..rows {
            coordinate[row] = rotated_vectors[row * DIMENSION + dim];
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

fn clipped_uniform_codebook(bit_width: usize) -> Vec<f32> {
    let levels = 1_usize << bit_width;
    let sigma = (DIMENSION as f32).sqrt().recip();
    let clip = 3.0 * sigma;
    (0..levels)
        .map(|code| {
            let midpoint = (code as f32 + 0.5) / levels as f32;
            midpoint.mul_add(2.0 * clip, -clip)
        })
        .collect()
}

fn normal_lloyd_codebook(bit_width: usize) -> Vec<f32> {
    let levels = 1_usize << bit_width;
    let sigma = (DIMENSION as f64).sqrt().recip();
    let spread = 3.0 * sigma;
    let mut centroids = (0..levels)
        .map(|code| -spread + 2.0 * spread * code as f64 / (levels - 1) as f64)
        .collect::<Vec<_>>();

    for _ in 0..64 {
        let boundaries = centroid_boundaries(&centroids);
        let mut max_change = 0.0f64;
        for code in 0..levels {
            let low = if code == 0 {
                f64::NEG_INFINITY
            } else {
                boundaries[code - 1]
            };
            let high = if code + 1 == levels {
                f64::INFINITY
            } else {
                boundaries[code]
            };
            let next = normal_interval_mean(low, high, sigma);
            max_change = max_change.max((centroids[code] - next).abs());
            centroids[code] = next;
        }
        if max_change < 1e-12 {
            break;
        }
    }

    centroids
        .into_iter()
        .map(|centroid| centroid as f32)
        .collect()
}

fn centroid_boundaries(centroids: &[f64]) -> Vec<f64> {
    centroids
        .windows(2)
        .map(|pair| (pair[0] + pair[1]) * 0.5)
        .collect()
}

fn normal_interval_mean(low: f64, high: f64, sigma: f64) -> f64 {
    let low_z = low / sigma;
    let high_z = high / sigma;
    let probability = standard_normal_cdf(high_z) - standard_normal_cdf(low_z);
    if probability <= 1e-15 {
        return (low + high) * 0.5;
    }
    sigma * (standard_normal_pdf(low_z) - standard_normal_pdf(high_z)) / probability
}

fn standard_normal_pdf(value: f64) -> f64 {
    const INV_SQRT_2_PI: f64 = 0.398_942_280_401_432_7;
    if value.is_infinite() {
        0.0
    } else {
        INV_SQRT_2_PI * (-0.5 * value * value).exp()
    }
}

fn standard_normal_cdf(value: f64) -> f64 {
    if value == f64::NEG_INFINITY {
        0.0
    } else if value == f64::INFINITY {
        1.0
    } else {
        0.5 * (1.0 + erf_approx(value / f64::sqrt(2.0)))
    }
}

fn erf_approx(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let polynomial =
        (((((1.061_405_429 * t - 1.453_152_027) * t + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t)
            * (-x * x).exp();
    sign * (1.0 - polynomial)
}

fn nearest_code(value: f32, codebook: &[f32]) -> usize {
    let mut best = 0;
    let mut best_distance = f32::INFINITY;
    for (code, centroid) in codebook.iter().enumerate() {
        let distance = (*centroid - value).abs();
        if distance
            .total_cmp(&best_distance)
            .then_with(|| code.cmp(&best))
            .is_lt()
        {
            best = code;
            best_distance = distance;
        }
    }
    best
}

fn rotate_unit_vector(vector: &VectorValue, output: &mut [f32]) -> f32 {
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
    hadamard_transform(output);
    let scale = (DIMENSION as f32).sqrt().recip();
    for value in output {
        *value *= scale;
    }
    length
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

fn write_code(
    codes: &mut [u8],
    row: usize,
    bytes_per_vector: usize,
    bit_width: usize,
    dim: usize,
    code: usize,
) {
    let bit_offset = row * bytes_per_vector * u8::BITS as usize + dim * bit_width;
    let byte = bit_offset / u8::BITS as usize;
    let shift = bit_offset % u8::BITS as usize;
    let mask = ((1_u16 << bit_width) - 1) << shift;
    let mut word = u16::from(codes[byte]);
    if byte + 1 < codes.len() {
        word |= u16::from(codes[byte + 1]) << u8::BITS;
    }
    word = (word & !mask) | ((code as u16) << shift);
    codes[byte] = (word & 0xff) as u8;
    if shift + bit_width > u8::BITS as usize {
        codes[byte + 1] = (word >> u8::BITS) as u8;
    }
}

fn read_code(
    codes: &[u8],
    row: usize,
    bytes_per_vector: usize,
    bit_width: usize,
    dim: usize,
) -> usize {
    let bit_offset = row * bytes_per_vector * u8::BITS as usize + dim * bit_width;
    let byte = bit_offset / u8::BITS as usize;
    let shift = bit_offset % u8::BITS as usize;
    let mut word = u16::from(codes[byte]);
    if byte + 1 < codes.len() {
        word |= u16::from(codes[byte + 1]) << u8::BITS;
    }
    let mask = (1_u16 << bit_width) - 1;
    usize::from((word >> shift) & mask)
}
