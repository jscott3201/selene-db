//! Safe TurboQuant codec primitives for compressed vector storage.
//!
//! This module owns the deterministic, dependency-free pieces that graph vector
//! indexes can reuse before they add index maintenance, persistence, or SIMD
//! search caches.

use std::error::Error;
use std::fmt;
use std::mem::size_of;

use crate::MAX_VECTOR_DIMENSION;

#[path = "turbo_quant/blocked.rs"]
mod blocked;

pub use blocked::{TURBO_QUANT_BLOCK_ROWS, TurboQuantBlockedCodes};

/// Result type for TurboQuant codec operations.
pub type TurboQuantCodecResult<T> = Result<T, TurboQuantCodecError>;

/// Errors returned by safe TurboQuant codec primitives.
#[derive(Clone, Debug, PartialEq)]
pub enum TurboQuantCodecError {
    /// Bit widths must be in the inclusive `2..=4` range.
    InvalidBitWidth {
        /// Rejected bit width.
        bits: u8,
    },
    /// Vector dimensions must be non-zero and no larger than
    /// [`MAX_VECTOR_DIMENSION`].
    InvalidDimension {
        /// Rejected dimension.
        dimension: usize,
        /// Maximum accepted dimension.
        max: usize,
    },
    /// Caller-supplied packed bytes did not match the codec shape.
    ByteLengthMismatch {
        /// Expected byte length for the requested packed-code operation.
        expected: usize,
        /// Actual byte length supplied by the caller.
        actual: usize,
    },
    /// Packed-code storage size overflowed `usize`.
    SizeOverflow,
    /// Requested row is outside the packed-code matrix.
    RowOutOfBounds {
        /// Requested row.
        row: usize,
        /// Number of rows stored.
        rows: usize,
    },
    /// Requested dimension is outside the packed-code matrix.
    DimensionOutOfBounds {
        /// Requested dimension.
        dimension: usize,
        /// Number of dimensions stored per row.
        dimensions: usize,
    },
    /// A code exceeded the active bit width's maximum representable code.
    InvalidCode {
        /// Rejected code.
        code: u8,
        /// Maximum accepted code for this bit width.
        max: u8,
    },
    /// A value that must be finite was not finite.
    NonFiniteValue {
        /// Non-finite value.
        value: f32,
    },
}

impl fmt::Display for TurboQuantCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBitWidth { bits } => {
                write!(f, "invalid TurboQuant bit width {bits}; expected 2..=4")
            }
            Self::InvalidDimension { dimension, max } => write!(
                f,
                "invalid TurboQuant dimension {dimension}; expected 1..={max}"
            ),
            Self::ByteLengthMismatch { expected, actual } => write!(
                f,
                "invalid TurboQuant packed byte length {actual}; expected {expected}"
            ),
            Self::SizeOverflow => write!(f, "TurboQuant packed-code size overflowed usize"),
            Self::RowOutOfBounds { row, rows } => {
                write!(f, "TurboQuant row {row} is out of bounds for {rows} rows")
            }
            Self::DimensionOutOfBounds {
                dimension,
                dimensions,
            } => write!(
                f,
                "TurboQuant dimension {dimension} is out of bounds for {dimensions} dimensions"
            ),
            Self::InvalidCode { code, max } => {
                write!(f, "TurboQuant code {code} exceeds maximum code {max}")
            }
            Self::NonFiniteValue { value } => {
                write!(f, "TurboQuant value must be finite, got {value}")
            }
        }
    }
}

impl Error for TurboQuantCodecError {}

/// Validated TurboQuant bit width.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TurboQuantBitWidth(u8);

impl TurboQuantBitWidth {
    /// Construct a validated TurboQuant bit width.
    ///
    /// # Errors
    ///
    /// Returns [`TurboQuantCodecError::InvalidBitWidth`] unless `bits` is in
    /// the inclusive `2..=4` range.
    pub const fn new(bits: u8) -> TurboQuantCodecResult<Self> {
        if bits >= 2 && bits <= 4 {
            Ok(Self(bits))
        } else {
            Err(TurboQuantCodecError::InvalidBitWidth { bits })
        }
    }

    /// Return the number of bits stored for each encoded coordinate.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Return the number of quantization levels represented by this bit width.
    #[must_use]
    pub const fn levels(self) -> usize {
        1_usize << self.0
    }

    /// Return the maximum representable code for this bit width.
    #[must_use]
    pub const fn max_code(self) -> u8 {
        (1_u8 << self.0) - 1
    }
}

impl TryFrom<u8> for TurboQuantBitWidth {
    type Error = TurboQuantCodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<TurboQuantBitWidth> for u8 {
    fn from(value: TurboQuantBitWidth) -> Self {
        value.bits()
    }
}

/// Deterministic TurboQuant scalar codebook family.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TurboQuantCodebookKind {
    /// Uniform centroids clipped to three standard deviations of
    /// `N(0, 1 / dimension)`.
    ClippedUniform,
    /// Lloyd-Max centroids for `N(0, 1 / dimension)`.
    NormalLloydMax,
}

/// Deterministic scalar codebook for one TurboQuant vector dimension.
#[derive(Clone, Debug, PartialEq)]
pub struct TurboQuantCodebook {
    kind: TurboQuantCodebookKind,
    bit_width: TurboQuantBitWidth,
    dimension: usize,
    centroids: Vec<f32>,
    boundaries: Vec<f32>,
}

impl TurboQuantCodebook {
    /// Build a deterministic codebook for `kind`, `bit_width`, and
    /// `dimension`.
    ///
    /// # Errors
    ///
    /// Returns an error when `dimension` is zero or exceeds
    /// [`MAX_VECTOR_DIMENSION`].
    pub fn new(
        kind: TurboQuantCodebookKind,
        bit_width: TurboQuantBitWidth,
        dimension: usize,
    ) -> TurboQuantCodecResult<Self> {
        validate_dimension(dimension)?;
        let centroids = match kind {
            TurboQuantCodebookKind::ClippedUniform => {
                clipped_uniform_centroids(bit_width, dimension)
            }
            TurboQuantCodebookKind::NormalLloydMax => {
                normal_lloyd_max_centroids(bit_width, dimension)
            }
        };
        let boundaries = centroid_boundaries(&centroids);
        Ok(Self {
            kind,
            bit_width,
            dimension,
            centroids,
            boundaries,
        })
    }

    /// Build a clipped-uniform codebook.
    ///
    /// # Errors
    ///
    /// Returns an error when `dimension` is zero or exceeds
    /// [`MAX_VECTOR_DIMENSION`].
    pub fn clipped_uniform(
        bit_width: TurboQuantBitWidth,
        dimension: usize,
    ) -> TurboQuantCodecResult<Self> {
        Self::new(TurboQuantCodebookKind::ClippedUniform, bit_width, dimension)
    }

    /// Build a normal Lloyd-Max codebook.
    ///
    /// # Errors
    ///
    /// Returns an error when `dimension` is zero or exceeds
    /// [`MAX_VECTOR_DIMENSION`].
    pub fn normal_lloyd_max(
        bit_width: TurboQuantBitWidth,
        dimension: usize,
    ) -> TurboQuantCodecResult<Self> {
        Self::new(TurboQuantCodebookKind::NormalLloydMax, bit_width, dimension)
    }

    /// Return the codebook family.
    #[must_use]
    pub const fn kind(&self) -> TurboQuantCodebookKind {
        self.kind
    }

    /// Return the bit width used by this codebook.
    #[must_use]
    pub const fn bit_width(&self) -> TurboQuantBitWidth {
        self.bit_width
    }

    /// Return the vector dimension this codebook was calibrated for.
    #[must_use]
    pub const fn dimension(&self) -> usize {
        self.dimension
    }

    /// Return the codebook centroids in ascending code order.
    #[must_use]
    pub fn centroids(&self) -> &[f32] {
        &self.centroids
    }

    /// Return midpoint boundaries between adjacent centroids.
    #[must_use]
    pub fn boundaries(&self) -> &[f32] {
        &self.boundaries
    }

    /// Return the centroid for `code`.
    ///
    /// # Errors
    ///
    /// Returns [`TurboQuantCodecError::InvalidCode`] when `code` exceeds this
    /// codebook's bit width.
    pub fn centroid(&self, code: u8) -> TurboQuantCodecResult<f32> {
        self.validate_code(code)?;
        Ok(self.centroids[usize::from(code)])
    }

    /// Quantize a finite scalar into a code by scanning codebook boundaries.
    ///
    /// Values equal to a boundary choose the lower code, matching the
    /// lower-code tie break used by exact nearest-centroid scans.
    ///
    /// # Errors
    ///
    /// Returns [`TurboQuantCodecError::NonFiniteValue`] for NaN or infinity.
    pub fn encode_scalar(&self, value: f32) -> TurboQuantCodecResult<u8> {
        if !value.is_finite() {
            return Err(TurboQuantCodecError::NonFiniteValue { value });
        }
        Ok(self
            .boundaries
            .partition_point(|boundary| value > *boundary) as u8)
    }

    /// Return an approximate heap allocation footprint for this codebook.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.centroids
            .len()
            .saturating_add(self.boundaries.len())
            .saturating_mul(size_of::<f32>())
    }

    fn validate_code(&self, code: u8) -> TurboQuantCodecResult<()> {
        let max = self.bit_width.max_code();
        if code <= max {
            Ok(())
        } else {
            Err(TurboQuantCodecError::InvalidCode { code, max })
        }
    }
}

/// Row-major packed TurboQuant coordinate codes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurboQuantPackedCodes {
    bit_width: TurboQuantBitWidth,
    dimensions: usize,
    rows: usize,
    bytes_per_row: usize,
    bytes: Vec<u8>,
}

impl TurboQuantPackedCodes {
    /// Allocate zero-filled packed-code storage.
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
        let byte_len = bytes_per_row
            .checked_mul(rows)
            .ok_or(TurboQuantCodecError::SizeOverflow)?;
        Ok(Self {
            bit_width,
            dimensions,
            rows,
            bytes_per_row,
            bytes: vec![0; byte_len],
        })
    }

    /// Build packed-code storage from existing bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when dimensions are invalid, the computed byte size
    /// overflows `usize`, or `bytes.len()` does not match the expected shape.
    pub fn from_bytes(
        bit_width: TurboQuantBitWidth,
        dimensions: usize,
        rows: usize,
        bytes: Vec<u8>,
    ) -> TurboQuantCodecResult<Self> {
        let bytes_per_row = bytes_per_row(bit_width, dimensions)?;
        let expected = bytes_per_row
            .checked_mul(rows)
            .ok_or(TurboQuantCodecError::SizeOverflow)?;
        if bytes.len() != expected {
            return Err(TurboQuantCodecError::ByteLengthMismatch {
                expected,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            bit_width,
            dimensions,
            rows,
            bytes_per_row,
            bytes,
        })
    }

    /// Return the bit width used by the packed codes.
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

    /// Return the byte stride for one row, including any trailing padding bits.
    #[must_use]
    pub const fn bytes_per_row(&self) -> usize {
        self.bytes_per_row
    }

    /// Return the packed backing bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume this storage and return the packed backing bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Return the packed-code byte footprint.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// Resize the row count while preserving existing packed rows.
    ///
    /// Newly added rows are zero-filled. Shrinking drops trailing rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the computed byte size overflows `usize`.
    pub fn resize_rows(&mut self, rows: usize) -> TurboQuantCodecResult<()> {
        let byte_len = self
            .bytes_per_row
            .checked_mul(rows)
            .ok_or(TurboQuantCodecError::SizeOverflow)?;
        self.bytes.resize(byte_len, 0);
        self.rows = rows;
        Ok(())
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
        let mut word = u16::from(self.bytes[byte]);
        if byte + 1 < self.bytes.len() {
            word |= u16::from(self.bytes[byte + 1]) << u8::BITS;
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
        let mut word = u16::from(self.bytes[byte]);
        if byte + 1 < self.bytes.len() {
            word |= u16::from(self.bytes[byte + 1]) << u8::BITS;
        }
        word = (word & !mask) | (u16::from(code) << shift);
        self.bytes[byte] = (word & 0xff) as u8;
        if shift + usize::from(self.bit_width.bits()) > u8::BITS as usize {
            self.bytes[byte + 1] = (word >> u8::BITS) as u8;
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
        if row >= self.rows {
            return Err(TurboQuantCodecError::RowOutOfBounds {
                row,
                rows: self.rows,
            });
        }
        if dimension >= self.dimensions {
            return Err(TurboQuantCodecError::DimensionOutOfBounds {
                dimension,
                dimensions: self.dimensions,
            });
        }
        let row_bits = row
            .checked_mul(self.bytes_per_row)
            .and_then(|offset| offset.checked_mul(u8::BITS as usize))
            .ok_or(TurboQuantCodecError::SizeOverflow)?;
        let dimension_bits = dimension
            .checked_mul(usize::from(self.bit_width.bits()))
            .ok_or(TurboQuantCodecError::SizeOverflow)?;
        row_bits
            .checked_add(dimension_bits)
            .ok_or(TurboQuantCodecError::SizeOverflow)
    }
}

fn validate_dimension(dimension: usize) -> TurboQuantCodecResult<()> {
    if dimension == 0 || dimension > MAX_VECTOR_DIMENSION {
        Err(TurboQuantCodecError::InvalidDimension {
            dimension,
            max: MAX_VECTOR_DIMENSION,
        })
    } else {
        Ok(())
    }
}

fn bytes_per_row(bit_width: TurboQuantBitWidth, dimensions: usize) -> TurboQuantCodecResult<usize> {
    validate_dimension(dimensions)?;
    let bits_per_row = dimensions
        .checked_mul(usize::from(bit_width.bits()))
        .ok_or(TurboQuantCodecError::SizeOverflow)?;
    bits_per_row
        .checked_add(u8::BITS as usize - 1)
        .map(|bits| bits / u8::BITS as usize)
        .ok_or(TurboQuantCodecError::SizeOverflow)
}

fn clipped_uniform_centroids(bit_width: TurboQuantBitWidth, dimension: usize) -> Vec<f32> {
    let levels = bit_width.levels();
    let sigma = (dimension as f32).sqrt().recip();
    let clip = 3.0 * sigma;
    (0..levels)
        .map(|code| {
            let midpoint = (code as f32 + 0.5) / levels as f32;
            midpoint.mul_add(2.0 * clip, -clip)
        })
        .collect()
}

fn normal_lloyd_max_centroids(bit_width: TurboQuantBitWidth, dimension: usize) -> Vec<f32> {
    let levels = bit_width.levels();
    let sigma = (dimension as f64).sqrt().recip();
    let spread = 3.0 * sigma;
    let mut centroids = (0..levels)
        .map(|code| -spread + 2.0 * spread * code as f64 / (levels - 1) as f64)
        .collect::<Vec<_>>();

    for _ in 0..64 {
        let boundaries = f64_centroid_boundaries(&centroids);
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

fn centroid_boundaries(centroids: &[f32]) -> Vec<f32> {
    centroids
        .windows(2)
        .map(|pair| (pair[0] + pair[1]) * 0.5)
        .collect()
}

fn f64_centroid_boundaries(centroids: &[f64]) -> Vec<f64> {
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

#[cfg(test)]
mod tests;
