use std::mem::size_of;

use selene_core::{VectorMetric, VectorTopK, VectorValue, exact_vector_top_k};

use super::DIMENSION;

#[derive(Clone, Copy, Debug)]
pub(crate) struct TurboQuantVariant {
    pub(crate) name: &'static str,
    pub(crate) bit_width: usize,
    pub(crate) candidates: usize,
}

#[derive(Debug)]
pub(crate) struct TurboQuantIndex {
    variant: TurboQuantVariant,
    bytes_per_vector: usize,
    codebook: Vec<f32>,
    scales: Vec<f32>,
    codes: Vec<u8>,
}

impl TurboQuantIndex {
    pub(crate) fn build(vectors: &[VectorValue], variant: TurboQuantVariant) -> Self {
        assert!(matches!(variant.bit_width, 2..=4));
        assert!(DIMENSION.is_power_of_two());
        let bytes_per_vector = DIMENSION * variant.bit_width / u8::BITS as usize;
        let codebook = spherical_codebook(variant.bit_width);
        let mut scales = Vec::with_capacity(vectors.len());
        let mut codes = vec![0; vectors.len() * bytes_per_vector];
        let mut rotated = vec![0.0; DIMENSION];
        for (row, vector) in vectors.iter().enumerate() {
            rotate_unit_vector(vector, &mut rotated);
            let mut reconstructed_inner = 0.0;
            for (dim, value) in rotated.iter().enumerate() {
                let code = nearest_code(*value, &codebook);
                reconstructed_inner += f64::from(*value) * f64::from(codebook[code]);
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
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in rows {
            candidates.push_distance(row, self.approx_distance(row, &rotated_query));
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
                .saturating_mul(size_of::<f32>()),
        )
    }

    fn approx_distance(&self, row: usize, rotated_query: &[f32]) -> f64 {
        let mut dot = 0.0;
        for (dim, query_component) in rotated_query.iter().enumerate() {
            let code = read_code(
                &self.codes,
                row,
                self.bytes_per_vector,
                self.variant.bit_width,
                dim,
            );
            dot += f64::from(*query_component) * f64::from(self.codebook[code]);
        }
        -(dot * f64::from(self.scales[row]))
    }
}

fn spherical_codebook(bit_width: usize) -> Vec<f32> {
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
