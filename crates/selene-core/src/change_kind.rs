//! Compact stable discriminants for [`Change`] variants.

use crate::Change;

/// Stable discriminant for one [`Change`] variant.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ChangeKind {
    /// [`Change::NodeCreated`].
    NodeCreated = 0,
    /// [`Change::NodeUpdated`].
    NodeUpdated = 1,
    /// [`Change::NodeDeleted`].
    NodeDeleted = 2,
    /// [`Change::EdgeCreated`].
    EdgeCreated = 3,
    /// [`Change::EdgeUpdated`].
    EdgeUpdated = 4,
    /// [`Change::EdgeDeleted`].
    EdgeDeleted = 5,
    /// [`Change::SchemaChanged`].
    SchemaChanged = 6,
    /// [`Change::NodePropertyRemoved`].
    NodePropertyRemoved = 7,
    /// [`Change::EdgePropertyRemoved`].
    EdgePropertyRemoved = 8,
    /// [`Change::NodeLabelRemoved`].
    NodeLabelRemoved = 9,
    /// [`Change::NodesOfTypeTruncated`].
    NodesOfTypeTruncated = 10,
    /// [`Change::EdgesOfTypeTruncated`].
    EdgesOfTypeTruncated = 11,
    /// [`Change::GraphReset`].
    GraphReset = 12,
}

impl Change {
    /// Return the compact kind discriminant for this change.
    #[must_use]
    pub const fn kind(&self) -> ChangeKind {
        match self {
            Self::NodeCreated { .. } => ChangeKind::NodeCreated,
            Self::NodeUpdated { .. } => ChangeKind::NodeUpdated,
            Self::NodeDeleted { .. } => ChangeKind::NodeDeleted,
            Self::EdgeCreated { .. } => ChangeKind::EdgeCreated,
            Self::EdgeUpdated { .. } => ChangeKind::EdgeUpdated,
            Self::EdgeDeleted { .. } => ChangeKind::EdgeDeleted,
            Self::SchemaChanged { .. } => ChangeKind::SchemaChanged,
            Self::NodePropertyRemoved { .. } => ChangeKind::NodePropertyRemoved,
            Self::EdgePropertyRemoved { .. } => ChangeKind::EdgePropertyRemoved,
            Self::NodeLabelRemoved { .. } => ChangeKind::NodeLabelRemoved,
            Self::NodesOfTypeTruncated { .. } => ChangeKind::NodesOfTypeTruncated,
            Self::EdgesOfTypeTruncated { .. } => ChangeKind::EdgesOfTypeTruncated,
            Self::GraphReset {} => ChangeKind::GraphReset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ChangeKind;
    use crate::Change;

    #[test]
    fn change_kind_maps_every_change_variant() {
        let expected = [
            ChangeKind::NodeCreated,
            ChangeKind::NodeUpdated,
            ChangeKind::NodeDeleted,
            ChangeKind::EdgeCreated,
            ChangeKind::EdgeUpdated,
            ChangeKind::EdgeDeleted,
            ChangeKind::SchemaChanged,
            ChangeKind::NodePropertyRemoved,
            ChangeKind::EdgePropertyRemoved,
            ChangeKind::NodeLabelRemoved,
            ChangeKind::NodesOfTypeTruncated,
            ChangeKind::EdgesOfTypeTruncated,
            ChangeKind::GraphReset,
        ];
        assert_eq!(Change::VARIANT_COUNT, expected.len());

        for (factory, kind) in Change::ALL.iter().zip(expected) {
            assert_eq!(factory().kind(), kind);
        }
    }

    #[test]
    fn change_kind_discriminants_are_contiguous_and_stable() {
        // The discriminant values are a stable telemetry/ABI surface; pin them
        // so a reordering of the `ChangeKind` enum is caught at test time.
        assert_eq!(ChangeKind::NodeCreated as u8, 0);
        assert_eq!(ChangeKind::NodePropertyRemoved as u8, 7);
        assert_eq!(ChangeKind::GraphReset as u8, 12);
    }
}
