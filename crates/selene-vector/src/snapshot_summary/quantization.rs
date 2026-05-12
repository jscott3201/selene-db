//! Quantization parity and stats rendering for vector snapshot fixtures.

use crate::{QuantMethod, QuantizationStats, QuantizationStatsKind};

use super::search::{SearchRowsSummary, render_search_rows};
use super::{format_score, quant_method_name};

/// Stable quantization stats summary.
#[derive(Clone, Debug, PartialEq)]
pub struct QuantizationStatsSummary {
    /// Quantization method.
    pub method: QuantMethod,
    /// Vector dimensionality.
    pub dim: usize,
    /// Number of quantized rows.
    pub code_count: usize,
    /// SQ8 code bytes.
    pub bytes_codes: usize,
    /// Method-specific byte accounting.
    pub kind: QuantizationStatsKind,
    /// Norm cache bytes.
    pub bytes_norms: usize,
    /// Compression ratio.
    pub compression_ratio: f32,
}

impl QuantizationStatsSummary {
    /// Convert provider stats into stable renderer input.
    #[must_use]
    pub fn from_stats(stats: QuantizationStats) -> Self {
        Self {
            method: stats.method,
            dim: stats.dim,
            code_count: stats.code_count,
            bytes_codes: stats.bytes_codes,
            kind: stats.kind,
            bytes_norms: stats.bytes_norms,
            compression_ratio: stats.compression_ratio,
        }
    }
}

/// Search results for exact, SQ8 asymmetric, and SQ8 rescored modes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuantizationParitySummary {
    /// Exact f32 baseline rows.
    pub f32_baseline: SearchRowsSummary,
    /// SQ8 asymmetric rows.
    pub sq8_asymmetric: SearchRowsSummary,
    /// SQ8 asymmetric plus f32 rescore rows.
    pub sq8_rescored: SearchRowsSummary,
}

pub(crate) fn render_stats(stats: &QuantizationStatsSummary, out: &mut Vec<String>) {
    match stats.kind {
        QuantizationStatsKind::Sq8 { bytes_ranges } => out.push(format!(
            "stats method={} dim={} code_count={} bytes_codes={} bytes_ranges={} bytes_norms={} compression_ratio={}",
            quant_method_name(stats.method),
            stats.dim,
            stats.code_count,
            stats.bytes_codes,
            bytes_ranges,
            stats.bytes_norms,
            format_score(stats.compression_ratio)
        )),
        QuantizationStatsKind::Pq { bytes_codebook } => out.push(format!(
            "stats method={} dim={} code_count={} bytes_codes={} bytes_codebook={} bytes_norms={} compression_ratio={}",
            quant_method_name(stats.method),
            stats.dim,
            stats.code_count,
            stats.bytes_codes,
            bytes_codebook,
            stats.bytes_norms,
            format_score(stats.compression_ratio)
        )),
    }
}

pub(crate) fn render_parity(parity: &QuantizationParitySummary, out: &mut Vec<String>) {
    render_search_rows("f32_baseline", &parity.f32_baseline, out);
    render_search_rows("sq8_asymmetric", &parity.sq8_asymmetric, out);
    render_search_rows("sq8_rescored", &parity.sq8_rescored, out);
}
