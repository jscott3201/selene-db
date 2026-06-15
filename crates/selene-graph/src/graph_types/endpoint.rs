//! Edge endpoint definitions for closed graph types.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Edge endpoint definition.
///
/// `OneOf` enumerates a sorted, deduplicated set of distinct node-type indices.
/// Construct it via [`EdgeEndpointDef::one_of`] to enforce the structural
/// invariants (sorted, deduplicated, length >= 2; singleton collapses to
/// [`EdgeEndpointDef::NodeType`]). Direct struct construction is permitted for
/// rkyv/serde decode paths but is rejected by
/// [`super::GraphTypeDef::validate_ref`] as defense in depth.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub enum EdgeEndpointDef {
    /// Accept any declared node type at this endpoint.
    Any,
    /// Require one concrete node-type index.
    NodeType(u32),
    /// Accept any node type drawn from a sorted, deduplicated, length >= 2 set
    /// of concrete node-type indices.
    OneOf(Vec<u32>),
}

impl EdgeEndpointDef {
    /// Construct an endpoint accepting `indices`, canonicalized.
    ///
    /// The input is sorted ascending and deduplicated. A single resulting
    /// index collapses to [`EdgeEndpointDef::NodeType`] so that `Eq`/`Hash`
    /// and rkyv byte encoding remain stable across construction paths.
    ///
    /// # Panics
    ///
    /// Panics when the resulting set is empty; zero-label endpoints are a
    /// caller bug and the upstream resolver must reject them before reaching
    /// this constructor.
    #[must_use]
    pub fn one_of(indices: impl IntoIterator<Item = u32>) -> Self {
        let mut buf: Vec<u32> = indices.into_iter().collect();
        buf.sort_unstable();
        buf.dedup();
        assert!(
            !buf.is_empty(),
            "EdgeEndpointDef::one_of called with empty index set"
        );
        match buf.len() {
            1 => Self::NodeType(buf[0]),
            _ => Self::OneOf(buf),
        }
    }

    /// Return true when this endpoint accepts the observed node-type index.
    #[must_use]
    pub fn matches_node_type(&self, node_type: u32) -> bool {
        match self {
            Self::Any => true,
            Self::NodeType(expected) => *expected == node_type,
            Self::OneOf(indices) => indices.binary_search(&node_type).is_ok(),
        }
    }

    /// Return true when two endpoint declarations can match a common node type.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Any, _) | (_, Self::Any) => true,
            (Self::NodeType(left), Self::NodeType(right)) => left == right,
            (Self::NodeType(index), Self::OneOf(indices))
            | (Self::OneOf(indices), Self::NodeType(index)) => indices.binary_search(index).is_ok(),
            (Self::OneOf(left), Self::OneOf(right)) => sorted_slices_intersect(left, right),
        }
    }

    /// Return the concrete node-type index when this endpoint resolves to
    /// exactly one node type.
    ///
    /// Returns `None` for [`EdgeEndpointDef::Any`] and
    /// [`EdgeEndpointDef::OneOf`]; callers that need to enumerate the
    /// candidate set should match the variant explicitly.
    #[must_use]
    pub const fn node_type_index(&self) -> Option<u32> {
        match self {
            Self::Any | Self::OneOf(_) => None,
            Self::NodeType(index) => Some(*index),
        }
    }
}

impl fmt::Display for EdgeEndpointDef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => formatter.write_str("Any"),
            Self::NodeType(index) => write!(formatter, "{index}"),
            Self::OneOf(indices) => {
                formatter.write_str("OneOf(")?;
                for (position, index) in indices.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{index}")?;
                }
                formatter.write_str(")")
            }
        }
    }
}

fn sorted_slices_intersect(left: &[u32], right: &[u32]) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}
