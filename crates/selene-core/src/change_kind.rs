//! Compact discriminants and filter sets for WAL change fan-out.

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
    /// [`Change::IndexExtensionEvent`].
    IndexExtensionEvent = 7,
}

/// Bit-set over [`ChangeKind`] discriminants.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ChangeKindSet(u8);

impl ChangeKindSet {
    /// Empty filter set.
    pub const EMPTY: Self = Self(0);

    /// Filter set containing every currently defined [`ChangeKind`].
    pub const ALL: Self = Self(0b1111_1111);

    /// Return true when `kind` is present in this set.
    #[must_use]
    pub const fn contains(self, kind: ChangeKind) -> bool {
        self.0 & kind_bit(kind) != 0
    }

    /// Return this set with `kind` included.
    #[must_use]
    pub const fn with(self, kind: ChangeKind) -> Self {
        Self(self.0 | kind_bit(kind))
    }

    /// Return the union of this set and `other`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
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
            Self::IndexExtensionEvent { .. } => ChangeKind::IndexExtensionEvent,
        }
    }
}

const fn kind_bit(kind: ChangeKind) -> u8 {
    1_u8 << (kind as u8)
}

#[cfg(test)]
mod tests {
    use super::{ChangeKind, ChangeKindSet};
    use crate::Change;

    static TOMBSTONE_KINDS: ChangeKindSet = ChangeKindSet::EMPTY
        .with(ChangeKind::NodeDeleted)
        .with(ChangeKind::EdgeDeleted);

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
            ChangeKind::IndexExtensionEvent,
        ];
        assert_eq!(Change::VARIANT_COUNT, expected.len());

        for (factory, kind) in Change::ALL.iter().zip(expected) {
            assert_eq!(factory().kind(), kind);
        }
    }

    #[test]
    fn change_kind_set_filters_kinds() {
        let node_mutations = ChangeKindSet::EMPTY
            .with(ChangeKind::NodeCreated)
            .with(ChangeKind::NodeUpdated);
        let deletions = ChangeKindSet::EMPTY
            .with(ChangeKind::NodeDeleted)
            .with(ChangeKind::EdgeDeleted);
        let combined = node_mutations.union(deletions);

        assert!(!ChangeKindSet::EMPTY.contains(ChangeKind::NodeCreated));
        assert!(ChangeKindSet::ALL.contains(ChangeKind::IndexExtensionEvent));
        assert!(combined.contains(ChangeKind::NodeCreated));
        assert!(combined.contains(ChangeKind::NodeUpdated));
        assert!(combined.contains(ChangeKind::NodeDeleted));
        assert!(combined.contains(ChangeKind::EdgeDeleted));
        assert!(!combined.contains(ChangeKind::EdgeCreated));
    }

    #[test]
    fn change_kind_set_supports_const_construction() {
        assert!(TOMBSTONE_KINDS.contains(ChangeKind::NodeDeleted));
        assert!(TOMBSTONE_KINDS.contains(ChangeKind::EdgeDeleted));
        assert!(!TOMBSTONE_KINDS.contains(ChangeKind::NodeUpdated));
    }

    #[test]
    fn default_is_empty() {
        assert_eq!(ChangeKindSet::default(), ChangeKindSet::EMPTY);
    }
}
