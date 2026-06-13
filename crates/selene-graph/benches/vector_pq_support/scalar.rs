use std::mem::size_of;

use selene_core::{VectorMetric, VectorTopK, VectorValue, exact_vector_top_k};

use super::DIMENSION;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScalarQuantVariant {
    pub(crate) name: &'static str,
    pub(crate) candidates: usize,
}

#[derive(Debug)]
pub(crate) struct ScalarQuantIndex {
    variant: ScalarQuantVariant,
    mins: Vec<f32>,
    scales: Vec<f32>,
    codes: Vec<u8>,
}

impl ScalarQuantIndex {
    pub(crate) fn build(vectors: &[VectorValue], variant: ScalarQuantVariant) -> Self {
        let (mins, scales) = scalar_ranges(vectors);
        let codes = encode_scalar_vectors(vectors, &mins, &scales);
        Self {
            variant,
            mins,
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
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in rows {
            candidates.push_distance(row, self.approx_distance(row, query));
        }
        self.exact_rerank(vectors, query, candidates, k)
    }

    pub(crate) fn search_all_code_l2(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        k: usize,
    ) -> Vec<usize> {
        self.search_rows_code_l2(vectors, query, 0..vectors.len(), k)
    }

    pub(crate) fn search_rows_code_l2(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        rows: impl IntoIterator<Item = usize>,
        k: usize,
    ) -> Vec<usize> {
        let query_codes = encode_scalar_query(query, &self.mins, &self.scales);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in rows {
            candidates.push_distance(row, self.code_distance(row, &query_codes));
        }
        self.exact_rerank(vectors, query, candidates, k)
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.codes.len().saturating_add(
            self.mins
                .len()
                .saturating_add(self.scales.len())
                .saturating_mul(size_of::<f32>()),
        )
    }

    fn exact_rerank(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        candidates: VectorTopK<usize>,
        k: usize,
    ) -> Vec<usize> {
        let candidate_ids = candidates
            .into_hits()
            .into_iter()
            .map(|hit| hit.key)
            .collect::<Vec<_>>();
        let hits = exact_vector_top_k(
            VectorMetric::SquaredEuclidean,
            query,
            candidate_ids.iter().map(|&row| (row, &vectors[row])),
            k,
        )
        .expect("scalar-quant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    fn approx_distance(&self, row: usize, query: &VectorValue) -> f64 {
        let code_offset = row * DIMENSION;
        query
            .as_slice()
            .iter()
            .enumerate()
            .map(|(dim, query_value)| {
                let code = f32::from(self.codes[code_offset + dim]);
                let decoded = self.mins[dim] + self.scales[dim] * code;
                let delta = f64::from(*query_value) - f64::from(decoded);
                delta * delta
            })
            .sum()
    }

    fn code_distance(&self, row: usize, query_codes: &[u8]) -> f64 {
        let code_offset = row * DIMENSION;
        let mut distance = 0u32;
        for (dim, query_code) in query_codes.iter().enumerate() {
            let delta = u32::from(self.codes[code_offset + dim].abs_diff(*query_code));
            distance += delta * delta;
        }
        f64::from(distance)
    }
}

fn scalar_ranges(vectors: &[VectorValue]) -> (Vec<f32>, Vec<f32>) {
    let mut mins = vec![f32::INFINITY; DIMENSION];
    let mut maxs = vec![f32::NEG_INFINITY; DIMENSION];
    for vector in vectors {
        for (dim, value) in vector.as_slice().iter().enumerate() {
            mins[dim] = mins[dim].min(*value);
            maxs[dim] = maxs[dim].max(*value);
        }
    }
    let scales = mins
        .iter()
        .zip(maxs)
        .map(|(min, max)| {
            let span = max - *min;
            if span > 0.0 { span / 255.0 } else { 1.0 }
        })
        .collect();
    (mins, scales)
}

fn encode_scalar_vectors(vectors: &[VectorValue], mins: &[f32], scales: &[f32]) -> Vec<u8> {
    let mut codes = vec![0; vectors.len() * DIMENSION];
    for (row, vector) in vectors.iter().enumerate() {
        for (dim, value) in vector.as_slice().iter().enumerate() {
            let scaled = ((*value - mins[dim]) / scales[dim]).round();
            codes[row * DIMENSION + dim] = scaled.clamp(0.0, 255.0) as u8;
        }
    }
    codes
}

fn encode_scalar_query(query: &VectorValue, mins: &[f32], scales: &[f32]) -> Vec<u8> {
    query
        .as_slice()
        .iter()
        .enumerate()
        .map(|(dim, value)| {
            let scaled = ((*value - mins[dim]) / scales[dim]).round();
            scaled.clamp(0.0, 255.0) as u8
        })
        .collect()
}
