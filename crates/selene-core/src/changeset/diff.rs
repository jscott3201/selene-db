//! Canonical label and property diffs carried by WAL changes.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::{CoreError, CoreResult, DbString, Value};

/// Label set difference.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelDiff {
    /// Labels added by the mutation.
    pub added: SmallVec<[DbString; 2]>,
    /// Labels removed by the mutation.
    pub removed: SmallVec<[DbString; 2]>,
}

impl LabelDiff {
    /// Construct a sorted, deduplicated label diff.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::OverlappingDiff`] when a label appears in both
    /// `added` and `removed`. Contradictory diffs would make WAL replay
    /// order-dependent, so the constructor refuses to build them.
    pub fn new(
        added: impl IntoIterator<Item = DbString>,
        removed: impl IntoIterator<Item = DbString>,
    ) -> CoreResult<Self> {
        let added = sorted_deduped(added);
        let removed = sorted_deduped(removed);
        ensure_disjoint("label", &added, &removed)?;
        Ok(Self { added, removed })
    }

    /// Return true if no labels changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Deserialize, Serialize)]
struct LabelDiffWire {
    added: SmallVec<[DbString; 2]>,
    removed: SmallVec<[DbString; 2]>,
}

impl Serialize for LabelDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Canonicalize on serialize. `LabelDiff::new` already sorts, so this is
        // a no-op (byte-identical) for constructed diffs. The fields are public,
        // so direct construction still emits canonical wire.
        let mut added = self.added.clone();
        let mut removed = self.removed.clone();
        added.sort_by(|lhs, rhs| lhs.as_str().cmp(rhs.as_str()));
        removed.sort_by(|lhs, rhs| lhs.as_str().cmp(rhs.as_str()));
        LabelDiffWire { added, removed }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LabelDiff {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Validate the canonical (strictly-ascending, dedup'd, disjoint)
        // invariant rather than re-sorting; a non-canonical payload is
        // rejected as malformed.
        let wire = LabelDiffWire::deserialize(deserializer)?;
        validate_sorted_unique(&wire.added, "LabelDiff.added")?;
        validate_sorted_unique(&wire.removed, "LabelDiff.removed")?;
        validate_disjoint(&wire.added, &wire.removed, "label")?;
        Ok(Self {
            added: wire.added,
            removed: wire.removed,
        })
    }
}

/// Property map difference.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDiff {
    /// Keys set to a new value. Use [`Value::Null`] for an explicit null set.
    pub set: SmallVec<[(DbString, Value); 4]>,
    /// Keys whose entries are removed entirely.
    pub removed: SmallVec<[DbString; 2]>,
}

impl PropertyDiff {
    /// Construct a sorted, deduplicated property diff.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::OverlappingDiff`] when a key appears in both `set`
    /// and `removed`. Contradictory diffs would make WAL replay
    /// order-dependent, so the constructor refuses to build them.
    pub fn new(
        set: impl IntoIterator<Item = (DbString, Value)>,
        removed: impl IntoIterator<Item = DbString>,
    ) -> CoreResult<Self> {
        let mut set: Vec<_> = set.into_iter().collect();
        set.sort_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
        set.dedup_by(|(lhs_key, lhs_value), (rhs_key, rhs_value)| {
            if lhs_key == rhs_key {
                *lhs_value = rhs_value.clone();
                true
            } else {
                false
            }
        });
        let set: SmallVec<[(DbString, Value); 4]> = set.into_iter().collect();
        let removed = sorted_deduped(removed);
        for (key, _) in set.iter() {
            if removed.binary_search(key).is_ok() {
                return Err(CoreError::OverlappingDiff {
                    kind: "property",
                    key: key.clone(),
                });
            }
        }
        Ok(Self { set, removed })
    }

    /// Return true if no properties changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.removed.is_empty()
    }
}

#[derive(Deserialize, Serialize)]
struct PropertyDiffWire {
    set: SmallVec<[(DbString, Value); 4]>,
    removed: SmallVec<[DbString; 2]>,
}

impl Serialize for PropertyDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Canonicalize on serialize. `PropertyDiff::new` already sorts, so this
        // is a no-op (byte-identical) for constructed diffs. The fields are
        // public, so direct construction still emits canonical wire.
        let mut set = self.set.clone();
        let mut removed = self.removed.clone();
        set.sort_by(|(lhs, _), (rhs, _)| lhs.as_str().cmp(rhs.as_str()));
        removed.sort_by(|lhs, rhs| lhs.as_str().cmp(rhs.as_str()));
        PropertyDiffWire { set, removed }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PropertyDiff {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Validate the canonical invariant (strictly-ascending set keys,
        // strictly-ascending removed, disjoint) rather than re-sorting; a
        // non-canonical payload is rejected as malformed.
        let wire = PropertyDiffWire::deserialize(deserializer)?;
        for window in wire.set.windows(2) {
            if window[0].0 >= window[1].0 {
                return Err(serde::de::Error::custom(
                    "PropertyDiff.set entries must be sorted by DbString order with no duplicate keys",
                ));
            }
        }
        validate_sorted_unique(&wire.removed, "PropertyDiff.removed")?;
        for (key, _) in wire.set.iter() {
            if wire.removed.binary_search(key).is_ok() {
                return Err(serde::de::Error::custom(format!(
                    "PropertyDiff: key {key} appears in both set and removed",
                )));
            }
        }
        Ok(Self {
            set: wire.set,
            removed: wire.removed,
        })
    }
}

fn sorted_deduped(values: impl IntoIterator<Item = DbString>) -> SmallVec<[DbString; 2]> {
    let mut values: SmallVec<[DbString; 2]> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values
}

fn ensure_disjoint(
    kind: &'static str,
    added: &SmallVec<[DbString; 2]>,
    removed: &SmallVec<[DbString; 2]>,
) -> CoreResult<()> {
    for label in added.iter() {
        if removed.binary_search(label).is_ok() {
            return Err(CoreError::OverlappingDiff {
                kind,
                key: label.clone(),
            });
        }
    }
    Ok(())
}

fn validate_sorted_unique<E: serde::de::Error>(
    values: &SmallVec<[DbString; 2]>,
    label: &'static str,
) -> Result<(), E> {
    for window in values.windows(2) {
        if window[0] >= window[1] {
            return Err(E::custom(format!(
                "{label} must be sorted by DbString order with no duplicates"
            )));
        }
    }
    Ok(())
}

fn validate_disjoint<E: serde::de::Error>(
    added: &SmallVec<[DbString; 2]>,
    removed: &SmallVec<[DbString; 2]>,
    kind: &'static str,
) -> Result<(), E> {
    for label in added.iter() {
        if removed.binary_search(label).is_ok() {
            return Err(E::custom(format!(
                "overlapping {kind} diff: {label} appears in both add/set and remove",
            )));
        }
    }
    Ok(())
}
