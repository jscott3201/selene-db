use super::{
    TurboQuantBitWidth, TurboQuantCodecError, TurboQuantCodecResult, TurboQuantPackedCodes,
    bytes_per_row, validate_dimension,
};

/// Number of vector rows stored together in blocked TurboQuant code storage.
pub const TURBO_QUANT_BLOCK_ROWS: usize = 32;

/// Block-major packed TurboQuant coordinate codes.
///
/// Rows are grouped in fixed 32-row blocks. For each block, all rows for packed
/// byte 0 are stored contiguously, then all rows for packed byte 1, and so on.
/// This preserves the same packed row representation as [`TurboQuantPackedCodes`]
/// while making search-time scans load one byte position across a whole block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurboQuantBlockedCodes {
    bit_width: TurboQuantBitWidth,
    dimensions: usize,
    rows: usize,
    bytes_per_row: usize,
    bytes: Vec<u8>,
}

impl TurboQuantBlockedCodes {
    /// Allocate zero-filled blocked-code storage.
    ///
    /// # Errors
    ///
    /// Returns an error when dimensions are invalid or the computed byte size
    /// overflows `usize`.
    pub fn new(
        bit_width: TurboQuantBitWidth,
        dimensions: usize,
        rows: usize,
    ) -> TurboQuantCodecResult<Self> {
        let bytes_per_row = bytes_per_row(bit_width, dimensions)?;
        let byte_len = byte_len(bytes_per_row, rows)?;
        Ok(Self {
            bit_width,
            dimensions,
            rows,
            bytes_per_row,
            bytes: vec![0; byte_len],
        })
    }

    /// Repack row-major packed codes into block-major storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the blocked storage size overflows `usize`.
    pub fn from_row_major(codes: &TurboQuantPackedCodes) -> TurboQuantCodecResult<Self> {
        let mut blocked = Self::new(codes.bit_width(), codes.dimensions(), codes.rows())?;
        for row in 0..codes.rows() {
            let source = row * codes.bytes_per_row();
            for byte in 0..codes.bytes_per_row() {
                blocked.set_row_byte(row, byte, codes.as_bytes()[source + byte]);
            }
        }
        Ok(blocked)
    }

    /// Return the bit width used by the blocked codes.
    #[must_use]
    pub const fn bit_width(&self) -> TurboQuantBitWidth {
        self.bit_width
    }

    /// Return the number of dimensions encoded in each row.
    #[must_use]
    pub const fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Return the number of encoded rows.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Return the byte stride for one packed row.
    #[must_use]
    pub const fn bytes_per_row(&self) -> usize {
        self.bytes_per_row
    }

    /// Return the number of row blocks in the backing storage.
    #[must_use]
    pub fn block_count(&self) -> usize {
        block_count(self.rows)
    }

    /// Return the number of real rows in `block`.
    #[must_use]
    pub fn block_len(&self, block: usize) -> usize {
        debug_assert!(block < self.block_count());
        let remaining = self.rows - block * TURBO_QUANT_BLOCK_ROWS;
        remaining.min(TURBO_QUANT_BLOCK_ROWS)
    }

    /// Return the packed bytes for one byte position across a 32-row block.
    ///
    /// The returned slice always has [`TURBO_QUANT_BLOCK_ROWS`] bytes. Tail
    /// lanes beyond [`Self::block_len`] are zero padding.
    #[must_use]
    pub fn block_byte(&self, block: usize, byte: usize) -> &[u8] {
        debug_assert!(block < self.block_count());
        debug_assert!(byte < self.bytes_per_row);
        let offset = (block * self.bytes_per_row + byte) * TURBO_QUANT_BLOCK_ROWS;
        &self.bytes[offset..offset + TURBO_QUANT_BLOCK_ROWS]
    }

    /// Return one packed row byte.
    ///
    /// # Errors
    ///
    /// Returns a bounds error when `row` or `byte` is outside the matrix.
    pub fn row_byte(&self, row: usize, byte: usize) -> TurboQuantCodecResult<u8> {
        let offset = self.byte_offset(row, byte)?;
        Ok(self.bytes[offset])
    }

    /// Return the blocked backing bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the blocked-code byte footprint.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// Resize the row count while preserving existing packed rows.
    ///
    /// Newly added rows are zero-filled. Shrinking clears rows that remain in a
    /// retained tail block, so later growth cannot expose stale packed bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the computed byte size overflows `usize`.
    pub fn resize_rows(&mut self, rows: usize) -> TurboQuantCodecResult<()> {
        validate_dimension(self.dimensions)?;
        let old_rows = self.rows;
        let byte_len = byte_len(self.bytes_per_row, rows)?;
        self.bytes.resize(byte_len, 0);
        for row in old_rows.min(rows)..old_rows.max(rows) {
            for byte in 0..self.bytes_per_row {
                if let Some(offset) = self.byte_offset_if_allocated(row, byte) {
                    self.bytes[offset] = 0;
                }
            }
        }
        self.rows = rows;
        Ok(())
    }

    /// Remove one row by moving the current last row into its slot.
    ///
    /// This preserves the packed row bytes without decoding individual
    /// coordinate codes. Tail bytes are cleared through [`Self::resize_rows`].
    ///
    /// # Errors
    ///
    /// Returns a bounds error when `row` is outside the matrix, or an overflow
    /// error if shrinking the storage would overflow internal size accounting.
    pub fn swap_remove_row(&mut self, row: usize) -> TurboQuantCodecResult<()> {
        self.validate_row(row)?;
        let last = self.rows - 1;
        if row != last {
            for byte in 0..self.bytes_per_row {
                let source = self.byte_offset_unchecked(last, byte);
                let destination = self.byte_offset_unchecked(row, byte);
                self.bytes[destination] = self.bytes[source];
            }
        }
        self.resize_rows(last)
    }

    /// Read one packed coordinate code.
    ///
    /// # Errors
    ///
    /// Returns bounds errors when `row` or `dimension` is outside the packed
    /// matrix.
    pub fn read(&self, row: usize, dimension: usize) -> TurboQuantCodecResult<u8> {
        let bit_offset = self.bit_offset(row, dimension)?;
        let byte = bit_offset / u8::BITS as usize;
        let shift = bit_offset % u8::BITS as usize;
        let mut word = u16::from(self.bytes[self.byte_offset(row, byte)?]);
        if byte + 1 < self.bytes_per_row {
            word |= u16::from(self.bytes[self.byte_offset(row, byte + 1)?]) << u8::BITS;
        }
        let mask = (1_u16 << self.bit_width.bits()) - 1;
        Ok(((word >> shift) & mask) as u8)
    }

    /// Write one packed coordinate code.
    ///
    /// # Errors
    ///
    /// Returns bounds errors when `row` or `dimension` is outside the packed
    /// matrix, and [`TurboQuantCodecError::InvalidCode`] when `code` exceeds
    /// this storage's bit width.
    pub fn write(&mut self, row: usize, dimension: usize, code: u8) -> TurboQuantCodecResult<()> {
        self.validate_code(code)?;
        let bit_offset = self.bit_offset(row, dimension)?;
        let byte = bit_offset / u8::BITS as usize;
        let shift = bit_offset % u8::BITS as usize;
        let mask = ((1_u16 << self.bit_width.bits()) - 1) << shift;
        let first = self.byte_offset(row, byte)?;
        let mut word = u16::from(self.bytes[first]);
        let second = (byte + 1 < self.bytes_per_row)
            .then(|| self.byte_offset(row, byte + 1))
            .transpose()?;
        if let Some(second) = second {
            word |= u16::from(self.bytes[second]) << u8::BITS;
        }
        word = (word & !mask) | (u16::from(code) << shift);
        self.bytes[first] = (word & 0xff) as u8;
        if shift + usize::from(self.bit_width.bits()) > u8::BITS as usize
            && let Some(second) = second
        {
            self.bytes[second] = (word >> u8::BITS) as u8;
        }
        Ok(())
    }

    fn validate_code(&self, code: u8) -> TurboQuantCodecResult<()> {
        let max = self.bit_width.max_code();
        if code <= max {
            Ok(())
        } else {
            Err(TurboQuantCodecError::InvalidCode { code, max })
        }
    }

    fn bit_offset(&self, row: usize, dimension: usize) -> TurboQuantCodecResult<usize> {
        self.validate_row(row)?;
        if dimension >= self.dimensions {
            return Err(TurboQuantCodecError::DimensionOutOfBounds {
                dimension,
                dimensions: self.dimensions,
            });
        }
        dimension
            .checked_mul(usize::from(self.bit_width.bits()))
            .ok_or(TurboQuantCodecError::SizeOverflow)
    }

    fn byte_offset(&self, row: usize, byte: usize) -> TurboQuantCodecResult<usize> {
        self.validate_row(row)?;
        if byte >= self.bytes_per_row {
            return Err(TurboQuantCodecError::DimensionOutOfBounds {
                dimension: byte.saturating_mul(u8::BITS as usize),
                dimensions: self.dimensions,
            });
        }
        Ok(self.byte_offset_unchecked(row, byte))
    }

    fn validate_row(&self, row: usize) -> TurboQuantCodecResult<()> {
        if row >= self.rows {
            Err(TurboQuantCodecError::RowOutOfBounds {
                row,
                rows: self.rows,
            })
        } else {
            Ok(())
        }
    }

    fn set_row_byte(&mut self, row: usize, byte: usize, value: u8) {
        let offset = self.byte_offset_unchecked(row, byte);
        self.bytes[offset] = value;
    }

    fn byte_offset_if_allocated(&self, row: usize, byte: usize) -> Option<usize> {
        let offset = self.byte_offset_unchecked(row, byte);
        (offset < self.bytes.len()).then_some(offset)
    }

    fn byte_offset_unchecked(&self, row: usize, byte: usize) -> usize {
        let block = row / TURBO_QUANT_BLOCK_ROWS;
        let lane = row % TURBO_QUANT_BLOCK_ROWS;
        (block * self.bytes_per_row + byte) * TURBO_QUANT_BLOCK_ROWS + lane
    }
}

fn block_count(rows: usize) -> usize {
    rows.div_ceil(TURBO_QUANT_BLOCK_ROWS)
}

fn byte_len(bytes_per_row: usize, rows: usize) -> TurboQuantCodecResult<usize> {
    block_count(rows)
        .checked_mul(bytes_per_row)
        .and_then(|bytes| bytes.checked_mul(TURBO_QUANT_BLOCK_ROWS))
        .ok_or(TurboQuantCodecError::SizeOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_codes_match_row_major_reads() {
        for bits in 2..=4 {
            let bit_width = TurboQuantBitWidth::new(bits).unwrap();
            let mut row_major = TurboQuantPackedCodes::new(bit_width, 11, 35).unwrap();
            let mut blocked = TurboQuantBlockedCodes::new(bit_width, 11, 35).unwrap();
            for row in 0..row_major.rows() {
                for dimension in 0..row_major.dimensions() {
                    let code = ((row * 3 + dimension) % bit_width.levels()) as u8;
                    row_major.write(row, dimension, code).unwrap();
                    blocked.write(row, dimension, code).unwrap();
                }
            }

            for row in 0..row_major.rows() {
                for dimension in 0..row_major.dimensions() {
                    assert_eq!(
                        blocked.read(row, dimension).unwrap(),
                        row_major.read(row, dimension).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn row_major_repack_uses_block_byte_layout() {
        let bit_width = TurboQuantBitWidth::new(4).unwrap();
        let mut row_major = TurboQuantPackedCodes::new(bit_width, 4, 35).unwrap();
        for row in 0..row_major.rows() {
            for dimension in 0..row_major.dimensions() {
                row_major
                    .write(row, dimension, ((row + dimension) % 16) as u8)
                    .unwrap();
            }
        }

        let blocked = TurboQuantBlockedCodes::from_row_major(&row_major).unwrap();

        assert_eq!(blocked.block_count(), 2);
        assert_eq!(blocked.block_len(0), TURBO_QUANT_BLOCK_ROWS);
        assert_eq!(blocked.block_len(1), 3);
        for byte in 0..row_major.bytes_per_row() {
            let block_byte = blocked.block_byte(0, byte);
            for (row, packed) in block_byte.iter().enumerate() {
                assert_eq!(
                    *packed,
                    row_major.as_bytes()[row * row_major.bytes_per_row() + byte]
                );
            }
        }
    }

    #[test]
    fn resize_rows_clears_retained_tail_slots() {
        let bit_width = TurboQuantBitWidth::new(4).unwrap();
        let mut blocked = TurboQuantBlockedCodes::new(bit_width, 2, 4).unwrap();
        blocked.write(3, 0, 15).unwrap();
        blocked.resize_rows(2).unwrap();
        blocked.resize_rows(4).unwrap();

        assert_eq!(blocked.read(3, 0).unwrap(), 0);
    }

    #[test]
    fn swap_remove_row_moves_last_row_and_clears_tail() {
        for bits in 2..=4 {
            let bit_width = TurboQuantBitWidth::new(bits).unwrap();
            let mut blocked = TurboQuantBlockedCodes::new(bit_width, 11, 35).unwrap();
            let last = blocked.rows() - 1;
            let removed = 7;
            let max_code = usize::from(bit_width.max_code());
            let moved_codes = (0..blocked.dimensions())
                .map(|dim| ((last * 5 + dim * 3) % (max_code + 1)) as u8)
                .collect::<Vec<_>>();
            for row in 0..blocked.rows() {
                for dim in 0..blocked.dimensions() {
                    let code = ((row * 5 + dim * 3) % (max_code + 1)) as u8;
                    blocked.write(row, dim, code).unwrap();
                }
            }

            blocked.swap_remove_row(removed).unwrap();

            assert_eq!(blocked.rows(), last);
            for (dim, expected) in moved_codes.into_iter().enumerate() {
                assert_eq!(blocked.read(removed, dim).unwrap(), expected);
            }
            blocked.resize_rows(last + 1).unwrap();
            for dim in 0..blocked.dimensions() {
                assert_eq!(blocked.read(last, dim).unwrap(), 0);
            }
        }
    }
}
