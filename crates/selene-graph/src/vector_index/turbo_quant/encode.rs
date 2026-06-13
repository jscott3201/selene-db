use super::{
    MIN_RECONSTRUCTED_INNER, TURBO_QUANT_BITS, TurboQuantVectorIndex, calibrate_value,
    codec_invariant, reconstruct_value,
};
use crate::error::GraphResult;

impl TurboQuantVectorIndex {
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
        self.codes
            .write_row_bytes(slot, row_bytes)
            .map_err(codec_invariant)?;
        self.row_scales[slot] = (1.0 / reconstructed_inner.max(MIN_RECONSTRUCTED_INNER)) as f32;
        Ok(())
    }

    fn encode_component(
        &self,
        value: f32,
        dimension: usize,
        reconstructed_inner: &mut f64,
    ) -> GraphResult<u8> {
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
        *reconstructed_inner += f64::from(value) * f64::from(reconstructed);
        Ok(code)
    }
}
