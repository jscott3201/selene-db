use std::array;

use selene_core::{
    TURBO_QUANT_BLOCK_ROWS, VectorMetric, VectorTopK, VectorValue, exact_vector_top_k,
};
use wide::{i16x8, u8x16, u16x16};

use super::{TurboQuantIndex, query_bias, query_component_for_score, rotate_unit_vector};

impl TurboQuantIndex {
    pub(super) fn search_all_blocked_fast_scan(
        &self,
        vectors: &[VectorValue],
        query: &VectorValue,
        k: usize,
    ) -> Vec<usize> {
        assert_eq!(query.dimension(), self.dimension);
        let mut rotated_query = vec![0.0; self.dimension];
        let _query_length = rotate_unit_vector(query, &mut rotated_query);
        let query_bias = query_bias(&rotated_query, &self.shift);
        let lut = self.fast_scan_lut(&rotated_query);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        let blocked = self
            .blocked_codes
            .as_ref()
            .expect("blocked fast-scan scorer builds blocked code storage");
        let mut accumulators = [u16x16::splat(0), u16x16::splat(0)];
        for block in 0..blocked.block_count() {
            let block_len = blocked.block_len(block);
            accumulators.fill(u16x16::splat(0));
            for (byte, byte_lut) in lut.bytes.iter().enumerate() {
                let codes = blocked.block_byte(block, byte);
                accumulate_half(&mut accumulators[0], load_lanes(&codes[..16]), byte_lut);
                accumulate_half(&mut accumulators[1], load_lanes(&codes[16..]), byte_lut);
            }
            let lanes = [accumulators[0].to_array(), accumulators[1].to_array()];
            let base_row = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let row = base_row + lane;
                let centered = i32::from(lanes[lane / 16][lane % 16]) - lut.zero_sum;
                let dot = query_bias + f64::from(centered) * lut.dequant;
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

    fn fast_scan_lut(&self, rotated_query: &[f32]) -> FastScanQueryLut {
        let quantized_components = self.bytes_per_vector * 2;
        assert!(
            quantized_components <= usize::from(u16::MAX) / 2,
            "FastScan benchmark u16 accumulation requires <= 32767 quantized components"
        );
        let quant_limit = ((usize::from(u16::MAX) / (quantized_components * 2))
            .min(i8::MAX as usize)
            .max(1)) as i16;
        let max_abs = self.max_query_contribution(rotated_query);
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
        let bytes = (0..self.bytes_per_vector)
            .map(|byte| {
                let first_dim = byte * 2;
                let second_dim = first_dim + 1;
                FastScanByteLut {
                    first: self.dimension_lut(first_dim, rotated_query, quant_scale, quant_limit),
                    second: self.dimension_lut(second_dim, rotated_query, quant_scale, quant_limit),
                }
            })
            .collect();
        FastScanQueryLut {
            bytes,
            zero_sum: (quantized_components as i32) * i32::from(quant_limit),
            dequant,
        }
    }

    fn max_query_contribution(&self, rotated_query: &[f32]) -> f64 {
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

    fn dimension_lut(
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
struct FastScanQueryLut {
    bytes: Vec<FastScanByteLut>,
    zero_sum: i32,
    dequant: f64,
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
        .expect("FastScan benchmark loads exactly sixteen lanes");
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
