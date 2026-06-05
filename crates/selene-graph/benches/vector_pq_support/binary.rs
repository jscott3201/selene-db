use std::mem::size_of;

use selene_core::{VectorMetric, VectorTopK, VectorValue, exact_vector_top_k};

use super::DIMENSION;

#[derive(Clone, Copy, Debug)]
pub(crate) struct BinaryQuantVariant {
    pub(crate) name: &'static str,
    pub(crate) candidates: usize,
}

#[derive(Debug)]
pub(crate) struct BinaryQuantIndex {
    variant: BinaryQuantVariant,
    words_per_vector: usize,
    codes: Vec<u64>,
}

impl BinaryQuantIndex {
    pub(crate) fn build(vectors: &[VectorValue], variant: BinaryQuantVariant) -> Self {
        let words_per_vector = DIMENSION.div_ceil(u64::BITS as usize);
        let mut codes = vec![0; vectors.len() * words_per_vector];
        for (row, vector) in vectors.iter().enumerate() {
            encode_binary_vector(
                vector,
                &mut codes[row * words_per_vector..(row + 1) * words_per_vector],
            );
        }
        Self {
            variant,
            words_per_vector,
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
        let query_code = binary_code(query, self.words_per_vector);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in rows {
            candidates.push_distance(row, f64::from(self.hamming_distance(row, &query_code)));
        }
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
        .expect("binary-quant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.codes.len().saturating_mul(size_of::<u64>())
    }

    fn hamming_distance(&self, row: usize, query_code: &[u64]) -> u32 {
        let offset = row * self.words_per_vector;
        self.codes[offset..offset + self.words_per_vector]
            .iter()
            .zip(query_code)
            .map(|(left, right)| (left ^ right).count_ones())
            .sum()
    }
}

fn binary_code(vector: &VectorValue, words_per_vector: usize) -> Vec<u64> {
    let mut words = vec![0; words_per_vector];
    encode_binary_vector(vector, &mut words);
    words
}

fn encode_binary_vector(vector: &VectorValue, words: &mut [u64]) {
    words.fill(0);
    for (dim, value) in vector.as_slice().iter().enumerate() {
        if *value >= 0.0 {
            words[dim / u64::BITS as usize] |= 1_u64 << (dim % u64::BITS as usize);
        }
    }
}
