use rayon::prelude::*;
use selene_core::TurboQuantCodebook;

use super::{
    MIN_RECONSTRUCTED_INNER, TURBO_QUANT_BITS, TurboQuantVectorIndex, calibrate_value,
    codec_invariant, reconstruct_value,
};
use crate::error::GraphResult;

#[cfg(not(test))]
const TURBO_QUANT_PARALLEL_ENCODE_MIN_VALUES: usize = 1_000_000;
#[cfg(test)]
const TURBO_QUANT_PARALLEL_ENCODE_MIN_VALUES: usize = 32;

impl TurboQuantVectorIndex {
    pub(super) fn encode_bulk_slots(&mut self, rotated: &[f32]) -> GraphResult<()> {
        if should_parallelize_bulk_encode(self.rows.len(), self.dimension) {
            return self.encode_bulk_slots_parallel(rotated);
        }
        let mut row_bytes = Vec::with_capacity(self.bytes_per_row);
        let context = EncodeContext::new(
            self.dimension,
            self.bytes_per_row,
            &self.codebook,
            &self.shift,
            &self.scale,
            &self.inv_scale,
        );
        for slot in 0..self.rows.len() {
            let start = slot * self.dimension;
            let end = start + self.dimension;
            let row_scale = context.encode_row_bytes(&rotated[start..end], &mut row_bytes)?;
            self.codes
                .write_row_bytes(slot, &row_bytes)
                .map_err(codec_invariant)?;
            self.row_scales[slot] = row_scale;
        }
        Ok(())
    }

    pub(super) fn encode_slot(&mut self, slot: usize, rotated: &[f32]) -> GraphResult<()> {
        let mut row_bytes = Vec::with_capacity(self.bytes_per_row);
        self.encode_slot_with_scratch(slot, rotated, &mut row_bytes)
    }

    pub(super) fn encode_slot_with_scratch(
        &mut self,
        slot: usize,
        rotated: &[f32],
        row_bytes: &mut Vec<u8>,
    ) -> GraphResult<()> {
        let context = EncodeContext::new(
            self.dimension,
            self.bytes_per_row,
            &self.codebook,
            &self.shift,
            &self.scale,
            &self.inv_scale,
        );
        let row_scale = context.encode_row_bytes(rotated, row_bytes)?;
        self.codes
            .write_row_bytes(slot, row_bytes)
            .map_err(codec_invariant)?;
        self.row_scales[slot] = row_scale;
        Ok(())
    }

    fn encode_bulk_slots_parallel(&mut self, rotated: &[f32]) -> GraphResult<()> {
        let context = EncodeContext::new(
            self.dimension,
            self.bytes_per_row,
            &self.codebook,
            &self.shift,
            &self.scale,
            &self.inv_scale,
        );
        let encoded = (0..self.rows.len())
            .into_par_iter()
            .map(|slot| {
                let start = slot * self.dimension;
                let end = start + self.dimension;
                let mut row_bytes = Vec::with_capacity(self.bytes_per_row);
                let row_scale = context.encode_row_bytes(&rotated[start..end], &mut row_bytes)?;
                Ok(EncodedRow {
                    row_bytes,
                    row_scale,
                })
            })
            .collect::<GraphResult<Vec<_>>>()?;

        for (slot, row) in encoded.iter().enumerate() {
            self.codes
                .write_row_bytes(slot, &row.row_bytes)
                .map_err(codec_invariant)?;
            self.row_scales[slot] = row.row_scale;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct EncodeContext<'a> {
    dimension: usize,
    bytes_per_row: usize,
    codebook: &'a TurboQuantCodebook,
    shift: &'a [f32],
    scale: &'a [f32],
    inv_scale: &'a [f32],
}

impl<'a> EncodeContext<'a> {
    const fn new(
        dimension: usize,
        bytes_per_row: usize,
        codebook: &'a TurboQuantCodebook,
        shift: &'a [f32],
        scale: &'a [f32],
        inv_scale: &'a [f32],
    ) -> Self {
        Self {
            dimension,
            bytes_per_row,
            codebook,
            shift,
            scale,
            inv_scale,
        }
    }

    fn encode_row_bytes(&self, rotated: &[f32], row_bytes: &mut Vec<u8>) -> GraphResult<f32> {
        debug_assert_eq!(self.codebook.bit_width().bits(), TURBO_QUANT_BITS);
        let mut reconstructed_inner = 0.0;
        row_bytes.clear();
        for byte in 0..self.bytes_per_row {
            let first_dimension = byte * 2;
            let first = self.encode_component(
                rotated[first_dimension],
                first_dimension,
                &mut reconstructed_inner,
            )?;
            let second_dimension = first_dimension + 1;
            let second = if second_dimension < self.dimension {
                self.encode_component(
                    rotated[second_dimension],
                    second_dimension,
                    &mut reconstructed_inner,
                )? << 4
            } else {
                0
            };
            row_bytes.push(first | second);
        }
        Ok((1.0 / reconstructed_inner.max(MIN_RECONSTRUCTED_INNER)) as f32)
    }

    fn encode_component(
        &self,
        value: f32,
        dimension: usize,
        reconstructed_inner: &mut f64,
    ) -> GraphResult<u8> {
        let calibrated = calibrate_value(value, dimension, self.shift, self.scale);
        let code = self
            .codebook
            .encode_scalar(calibrated)
            .map_err(codec_invariant)?;
        let reconstructed = reconstruct_value(
            usize::from(code),
            dimension,
            self.codebook.centroids(),
            self.shift,
            self.inv_scale,
        );
        *reconstructed_inner += f64::from(value) * f64::from(reconstructed);
        Ok(code)
    }
}

struct EncodedRow {
    row_bytes: Vec<u8>,
    row_scale: f32,
}

fn should_parallelize_bulk_encode(rows: usize, dimension: usize) -> bool {
    rows.saturating_mul(dimension) >= TURBO_QUANT_PARALLEL_ENCODE_MIN_VALUES
        && rayon::current_num_threads() > 1
}
