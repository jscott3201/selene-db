//! Shared zstd compression helpers for persistence payloads.

use std::io::Read;

use crate::{PersistError, PersistResult};

/// Compress bytes with zstd at the requested level.
pub(crate) fn compress_zstd(bytes: &[u8], level: i32) -> PersistResult<Vec<u8>> {
    zstd::stream::encode_all(bytes, level)
        .map_err(|error| PersistError::Compression(error.to_string()))
}

pub(crate) struct ZstdCompressor {
    inner: zstd::bulk::Compressor<'static>,
}

impl ZstdCompressor {
    pub(crate) fn new(level: i32) -> PersistResult<Self> {
        let inner = zstd::bulk::Compressor::new(level)
            .map_err(|error| PersistError::Compression(error.to_string()))?;
        Ok(Self { inner })
    }

    pub(crate) fn compress(&mut self, bytes: &[u8]) -> PersistResult<Vec<u8>> {
        self.inner
            .compress(bytes)
            .map_err(|error| PersistError::Compression(error.to_string()))
    }
}

/// Stream-decompress a zstd frame while bounding the decompressed output.
///
/// The `too_large` callback maps an overflowing frame into the caller's
/// domain-specific cap error.
pub(crate) fn decompress_zstd_bounded(
    bytes: &[u8],
    max_len: usize,
    too_large: impl Fn(usize, usize) -> PersistError,
) -> PersistResult<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(bytes)
        .map_err(|error| PersistError::Compression(error.to_string()))?;
    let mut output = Vec::new();
    let mut buf = [0_u8; 8 * 1024];
    loop {
        let read = decoder
            .read(&mut buf)
            .map_err(|error| PersistError::Compression(error.to_string()))?;
        if read == 0 {
            return Ok(output);
        }
        let next_len = output.len().saturating_add(read);
        if next_len > max_len {
            return Err(too_large(next_len, max_len));
        }
        output.extend_from_slice(&buf[..read]);
    }
}
