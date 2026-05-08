//! Hybrid logical clock timestamp per spec 04 section 3.2.

use serde::{Deserialize, Serialize};

/// NTP64-style hybrid logical clock timestamp.
///
/// Ordering is lexicographic by `(seconds, subseconds)`, which is the natural
/// total order for the persisted WAL header fields.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct HlcTimestamp {
    /// NTP64 seconds component.
    pub seconds: u64,
    /// NTP64 subseconds component.
    pub subseconds: u32,
}

impl HlcTimestamp {
    /// Construct a timestamp from its NTP64 components.
    #[must_use]
    pub const fn new(seconds: u64, subseconds: u32) -> Self {
        Self {
            seconds,
            subseconds,
        }
    }

    /// The zero timestamp.
    #[must_use]
    pub const fn zero() -> Self {
        Self::new(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_sets_components() {
        let timestamp = HlcTimestamp::new(42, 7);
        assert_eq!(timestamp.seconds, 42);
        assert_eq!(timestamp.subseconds, 7);
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(HlcTimestamp::default(), HlcTimestamp::zero());
    }

    #[test]
    fn ordering_is_component_order() {
        assert!(HlcTimestamp::new(1, 0) < HlcTimestamp::new(1, 1));
        assert!(HlcTimestamp::new(1, u32::MAX) < HlcTimestamp::new(2, 0));
    }

    #[test]
    fn postcard_round_trip_preserves_components() {
        let timestamp = HlcTimestamp::new(123, 456);
        let bytes = postcard::to_allocvec(&timestamp).unwrap();
        let decoded: HlcTimestamp = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, timestamp);
    }
}
