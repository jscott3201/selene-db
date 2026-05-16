//! Low-level PQ encode/decode and ADC LUT helpers.

use crate::DistanceMetric;
use crate::clustering::{nearest_centroid, squared_l2};
use crate::hnsw::distance::dot_product;

pub(super) fn encode_row(
    row: &[f32],
    codebook: &[f32],
    m: usize,
    k: usize,
    subdim: usize,
    out: &mut Vec<u8>,
) {
    for subspace in 0..m {
        let start = subspace * subdim;
        let codebook_start = subspace * k * subdim;
        let code = nearest_centroid(
            &row[start..start + subdim],
            &codebook[codebook_start..codebook_start + (k * subdim)],
            k,
            subdim,
        );
        out.push(u8::try_from(code).expect("version-1 PQ uses 256 centroids"));
    }
}

pub(super) fn build_query_lut_into(
    query: &[f32],
    codebook: &[f32],
    m: usize,
    k: usize,
    subdim: usize,
    metric: DistanceMetric,
    out: &mut Vec<f32>,
) {
    out.resize(m * k, 0.0);
    for subspace in 0..m {
        let query_start = subspace * subdim;
        let query_slice = &query[query_start..query_start + subdim];
        for centroid in 0..k {
            let center_start = ((subspace * k) + centroid) * subdim;
            let center = &codebook[center_start..center_start + subdim];
            out[(subspace * k) + centroid] = match metric {
                DistanceMetric::Cosine | DistanceMetric::Dot => dot_product(query_slice, center),
                DistanceMetric::L2 => squared_l2(query_slice, center),
            };
        }
    }
}

pub(super) fn lut_sum_for_codes(lut: &[f32], codes: &[u8], m: usize, k: usize) -> Option<f32> {
    if codes.len() != m || lut.len() != m.checked_mul(k)? {
        return None;
    }
    Some(
        codes
            .iter()
            .copied()
            .enumerate()
            .map(|(subspace, code)| lut[(subspace * k) + usize::from(code)])
            .sum(),
    )
}

pub(super) fn decode_codes(
    codebook: &[f32],
    codes: &[u8],
    m: usize,
    k: usize,
    subdim: usize,
    out: &mut [f32],
) -> Option<()> {
    if codes.len() != m || out.len() != m.checked_mul(subdim)? {
        return None;
    }
    for (subspace, code) in codes.iter().copied().enumerate() {
        let out_start = subspace * subdim;
        let center_start = ((subspace * k) + usize::from(code)) * subdim;
        out[out_start..out_start + subdim]
            .copy_from_slice(&codebook[center_start..center_start + subdim]);
    }
    Some(())
}
