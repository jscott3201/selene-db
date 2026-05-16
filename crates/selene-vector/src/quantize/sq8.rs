//! Per-coordinate scalar quantization (SQ8).

use rkyv::{Archive, Deserialize, Serialize};

use crate::hnsw::distance::dot_product;
use crate::{DistanceMetric, VectorError, snapshot};

use super::{QuantMethod, QuantizationStats, QuantizationStatsKind};

const SQ8_LEVELS: f32 = 255.0;

/// Derived per-coordinate range for SQ8.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PerCoordRange {
    pub(crate) min: Vec<f32>,
    pub(crate) max: Vec<f32>,
}

/// SQ8 quantized store in dense InternalIndex order.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QuantizedStoreSq8 {
    pub(crate) range: PerCoordRange,
    pub(crate) codes: Vec<u8>,
    pub(crate) approx_norms: Vec<f32>,
}

impl QuantizedStoreSq8 {
    pub(crate) fn build<'a>(
        node_count: usize,
        dim: usize,
        vectors: impl Iterator<Item = &'a [f32]>,
    ) -> Result<Self, VectorError> {
        let rows = vectors.collect::<Vec<_>>();
        if rows.len() != node_count {
            return Err(snapshot::encode_failed(
                snapshot::QUNT,
                format!("expected {node_count} vectors, observed {}", rows.len()),
            ));
        }

        if node_count == 0 {
            return Ok(Self {
                range: PerCoordRange {
                    min: vec![0.0; dim],
                    max: vec![0.0; dim],
                },
                codes: Vec::new(),
                approx_norms: Vec::new(),
            });
        }

        let mut min = vec![f32::INFINITY; dim];
        let mut max = vec![f32::NEG_INFINITY; dim];
        for row in &rows {
            validate_row(row, dim)?;
            for (coord, value) in row.iter().copied().enumerate() {
                min[coord] = min[coord].min(value);
                max[coord] = max[coord].max(value);
            }
        }

        let mut codes = Vec::with_capacity(node_count * dim);
        for row in &rows {
            for (coord, value) in row.iter().copied().enumerate() {
                codes.push(encode_component(value, min[coord], max[coord]));
            }
        }

        let mut store = Self {
            range: PerCoordRange { min, max },
            codes,
            approx_norms: Vec::with_capacity(node_count),
        };
        let mut scratch = vec![0.0; dim];
        for node_idx in 0..node_count {
            store.dequantize(node_idx, &mut scratch);
            store
                .approx_norms
                .push(dot_product(&scratch, &scratch).sqrt());
        }

        Ok(store)
    }

    pub(crate) fn node_count(&self) -> usize {
        self.approx_norms.len()
    }

    pub(crate) fn dim(&self) -> usize {
        self.range.min.len()
    }

    pub(crate) fn bytes_codes(&self) -> usize {
        self.codes.len()
    }

    pub(crate) fn bytes_ranges(&self) -> usize {
        (self.range.min.len() + self.range.max.len()) * std::mem::size_of::<f32>()
    }

    pub(crate) fn bytes_norms(&self) -> usize {
        self.approx_norms.len() * std::mem::size_of::<f32>()
    }

    pub(crate) fn stats(&self) -> QuantizationStats {
        let f32_bytes = self
            .node_count()
            .saturating_mul(self.dim())
            .saturating_mul(std::mem::size_of::<f32>());
        let quantized_bytes = self
            .bytes_codes()
            .saturating_add(self.bytes_ranges())
            .saturating_add(self.bytes_norms());
        let compression_ratio = if quantized_bytes == 0 {
            0.0
        } else {
            f32_bytes as f32 / quantized_bytes as f32
        };

        QuantizationStats {
            method: QuantMethod::Sq8,
            dim: self.dim(),
            code_count: self.node_count(),
            bytes_codes: self.bytes_codes(),
            kind: QuantizationStatsKind::Sq8 {
                bytes_ranges: self.bytes_ranges(),
            },
            bytes_norms: self.bytes_norms(),
            compression_ratio,
        }
    }

    pub(crate) fn dequantize(&self, node_idx: usize, out: &mut [f32]) {
        debug_assert_eq!(
            out.len(),
            self.dim(),
            "dequantize output dimension mismatch"
        );
        dequantize_row(&self.range, &self.codes, self.dim(), node_idx, out);
    }

    pub(crate) fn build_query_lut(&self, query: &[f32], metric: DistanceMetric) -> Vec<f32> {
        debug_assert_eq!(query.len(), self.dim(), "query LUT dimension mismatch");
        let dim = self.dim();
        let mut lut = vec![0.0; dim * 256];
        for coord in 0..dim {
            for code in 0..=u8::MAX {
                let dequantized =
                    dequantize_component(code, self.range.min[coord], self.range.max[coord]);
                let contribution = match metric {
                    DistanceMetric::Cosine | DistanceMetric::Dot => query[coord] * dequantized,
                    DistanceMetric::L2 => {
                        let delta = query[coord] - dequantized;
                        delta * delta
                    }
                };
                lut[(coord * 256) + usize::from(code)] = contribution;
            }
        }
        lut
    }

    pub(crate) fn lut_sum(&self, lut: &[f32], node_idx: usize) -> Option<f32> {
        let dim = self.dim();
        if node_idx >= self.node_count() || lut.len() != dim * 256 {
            return None;
        }
        let row_start = node_idx.checked_mul(dim)?;
        let row = self.codes.get(row_start..row_start.checked_add(dim)?)?;
        Some(
            row.iter()
                .copied()
                .enumerate()
                .map(|(coord, code)| lut[(coord * 256) + usize::from(code)])
                .sum(),
        )
    }

    pub(crate) fn approx_norm(&self, node_idx: usize) -> Option<f32> {
        self.approx_norms.get(node_idx).copied()
    }
}

fn validate_row(row: &[f32], dim: usize) -> Result<(), VectorError> {
    if row.len() != dim {
        return Err(snapshot::encode_failed(
            snapshot::QUNT,
            format!(
                "vector dimension {} disagrees with expected {dim}",
                row.len()
            ),
        ));
    }
    for (coord, value) in row.iter().copied().enumerate() {
        debug_assert!(
            value.is_finite(),
            "wire validators are authoritative for finite vector components"
        );
        if !value.is_finite() {
            return Err(snapshot::encode_failed(
                snapshot::QUNT,
                format!("non-finite vector component at coordinate {coord}: {value}"),
            ));
        }
    }
    Ok(())
}

fn encode_component(value: f32, min: f32, max: f32) -> u8 {
    if min == max {
        return 0;
    }
    (((value - min) / (max - min)) * SQ8_LEVELS)
        .round()
        .clamp(0.0, SQ8_LEVELS) as u8
}

fn dequantize_component(code: u8, min: f32, max: f32) -> f32 {
    if min == max {
        return min;
    }
    ((f32::from(code) / SQ8_LEVELS) * (max - min)) + min
}

fn dequantize_row(
    range: &PerCoordRange,
    codes: &[u8],
    dim: usize,
    node_idx: usize,
    out: &mut [f32],
) {
    let start = node_idx * dim;
    let row = &codes[start..start + dim];
    for (coord, code) in row.iter().copied().enumerate() {
        out[coord] = dequantize_component(code, range.min[coord], range.max[coord]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(rows: &[&[f32]]) -> QuantizedStoreSq8 {
        QuantizedStoreSq8::build(rows.len(), rows[0].len(), rows.iter().copied())
            .expect("SQ8 build succeeds")
    }

    #[test]
    fn sq8_roundtrips_at_range_endpoints() {
        let store = build(&[&[-2.0, 10.0], &[2.0, 20.0]]);
        let mut out = vec![0.0; 2];

        store.dequantize(0, &mut out);
        assert_eq!(out, vec![-2.0, 10.0]);
        store.dequantize(1, &mut out);
        assert_eq!(out, vec![2.0, 20.0]);
    }

    #[test]
    fn sq8_quantization_error_bounded() {
        let store = build(&[&[-1.0], &[1.0], &[0.3]]);
        let mut out = vec![0.0; 1];
        store.dequantize(2, &mut out);

        let bound = (2.0 / SQ8_LEVELS) / 2.0;
        assert!((out[0] - 0.3).abs() <= bound);
    }

    #[test]
    fn sq8_zero_span_coordinate_round_trips_exactly() {
        let store = build(&[&[3.5, -1.0], &[3.5, 1.0]]);
        let mut out = vec![0.0; 2];

        store.dequantize(0, &mut out);

        assert_eq!(out[0].to_bits(), 3.5f32.to_bits());
        assert_eq!(store.codes[0], 0);
    }

    #[test]
    fn lut_matches_direct_distance() {
        let store = build(&[&[-1.0, 2.0], &[1.0, 4.0]]);
        let query = [0.5, 3.0];
        let mut decoded = vec![0.0; 2];
        store.dequantize(1, &mut decoded);

        let l2_lut = store.build_query_lut(&query, DistanceMetric::L2);
        let dot_lut = store.build_query_lut(&query, DistanceMetric::Dot);

        let l2_sum = store.lut_sum(&l2_lut, 1).unwrap();
        let dot_sum = store.lut_sum(&dot_lut, 1).unwrap();
        let direct_l2: f32 = query
            .iter()
            .zip(&decoded)
            .map(|(a, b)| {
                let delta = a - b;
                delta * delta
            })
            .sum();
        let direct_dot = dot_product(&query, &decoded);

        assert!((l2_sum - direct_l2).abs() <= f32::EPSILON);
        assert!((dot_sum - direct_dot).abs() <= f32::EPSILON);
    }

    #[test]
    fn approx_norm_uses_dequantized_values() {
        let store = build(&[&[0.0], &[1.0], &[0.3]]);
        let mut decoded = vec![0.0; 1];
        store.dequantize(2, &mut decoded);

        let norm = store.approx_norm(2).unwrap();

        assert_ne!(decoded[0].to_bits(), 0.3_f32.to_bits());
        assert_eq!(norm.to_bits(), decoded[0].abs().to_bits());
    }

    #[test]
    fn empty_graph_build_uses_zero_ranges() {
        let store =
            QuantizedStoreSq8::build(0, 3, std::iter::empty()).expect("empty SQ8 store builds");

        assert_eq!(store.range.min, vec![0.0; 3]);
        assert_eq!(store.range.max, vec![0.0; 3]);
        assert!(store.codes.is_empty());
        assert!(store.approx_norms.is_empty());
    }
}
