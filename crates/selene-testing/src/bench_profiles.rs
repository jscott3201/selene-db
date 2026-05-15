//! Benchmark profile envelopes shared by Criterion and iai-callgrind benches.

use crate::bench_fixtures::BENCHMARK_SCALES;

/// Benchmark scale envelope selected by `SELENE_BENCH_PROFILE`.
///
/// Unknown values intentionally fall back to [`BenchProfile::Quick`] so CI
/// remains fast when the environment is unset or misspelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BenchProfile {
    /// CI default: one 1k-node scale.
    Quick,
    /// Publish-quality release-prep scales.
    Full,
    /// Nightly stress tracking: adds 250k.
    Stress,
}

impl BenchProfile {
    /// Resolve from `SELENE_BENCH_PROFILE`, defaulting to [`Self::Quick`].
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var("SELENE_BENCH_PROFILE")
            .ok()
            .as_deref()
            .map(profile_from_str)
            .unwrap_or(Self::Quick)
    }

    /// Return this profile's node scales in ascending order.
    #[must_use]
    pub const fn scales(self) -> &'static [usize] {
        match self {
            Self::Quick => &[1_000],
            Self::Full => BENCHMARK_SCALES,
            Self::Stress => &[1_000, 10_000, 50_000, 100_000, 250_000],
        }
    }

    /// Reduced sample count used by the bench binaries for quick feedback.
    #[must_use]
    pub const fn sample_size(self) -> usize {
        match self {
            Self::Quick => 10,
            Self::Full => 30,
            Self::Stress => 30,
        }
    }
}

fn profile_from_str(value: &str) -> BenchProfile {
    if value.eq_ignore_ascii_case("full") {
        BenchProfile::Full
    } else if value.eq_ignore_ascii_case("stress") {
        BenchProfile::Stress
    } else {
        BenchProfile::Quick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_profile_full_uses_canonical_scales() {
        assert_eq!(BenchProfile::Quick.scales(), &[1_000]);
        assert_eq!(BenchProfile::Full.scales(), BENCHMARK_SCALES);
        assert_eq!(
            BenchProfile::Stress.scales(),
            &[1_000, 10_000, 50_000, 100_000, 250_000]
        );
    }

    #[test]
    fn bench_profile_resolution_is_case_insensitive() {
        assert_eq!(profile_from_str("quick"), BenchProfile::Quick);
        assert_eq!(profile_from_str("FULL"), BenchProfile::Full);
        assert_eq!(profile_from_str("Stress"), BenchProfile::Stress);
        assert_eq!(profile_from_str("unknown"), BenchProfile::Quick);
    }
}
