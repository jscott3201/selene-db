//! Sorted label sets per spec 02 section 5.3.
//!
//! Labels are ordered by [`IStr`] interner-key order. The inline capacity of 3
//! matches the common node-label case; larger sets spill cleanly to the heap.
//! Edges semantically carry exactly one label, but that constraint is enforced
//! by `selene-graph`, not by this plain set type.

use serde::{Deserialize, Deserializer, Serialize};
use smallvec::SmallVec;

use crate::IStr;

/// Sorted set of graph labels.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct LabelSet(SmallVec<[IStr; 3]>);

impl LabelSet {
    /// Construct an empty label set.
    #[must_use]
    pub fn new() -> Self {
        Self(SmallVec::new())
    }

    /// Construct a label set from an iterator, sorting and deduplicating.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn from_iter(labels: impl IntoIterator<Item = IStr>) -> Self {
        labels.into_iter().collect()
    }

    /// Construct a single-label set.
    #[must_use]
    pub fn single(label: IStr) -> Self {
        let mut labels = SmallVec::new();
        labels.push(label);
        Self(labels)
    }

    /// Construct the one-label view used by edge APIs.
    #[must_use]
    pub fn edge(label: IStr) -> Self {
        Self::single(label)
    }

    /// Insert a label, returning true when it was not already present.
    pub fn insert(&mut self, label: IStr) -> bool {
        match self.0.binary_search(&label) {
            Ok(_) => false,
            Err(idx) => {
                self.0.insert(idx, label);
                true
            }
        }
    }

    /// Remove a label, returning true when it was present.
    pub fn remove(&mut self, label: &IStr) -> bool {
        match self.0.binary_search(label) {
            Ok(idx) => {
                self.0.remove(idx);
                true
            }
            Err(_) => false,
        }
    }

    /// Return true if `label` is present.
    #[must_use]
    pub fn contains(&self, label: &IStr) -> bool {
        self.0.binary_search(label).is_ok()
    }

    /// Number of labels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Return true if no labels are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate labels in sorted `IStr` order.
    pub fn iter(&self) -> impl Iterator<Item = &IStr> {
        self.0.iter()
    }

    #[cfg(test)]
    fn sorted_deduped_invariant_holds(&self) -> bool {
        self.0.windows(2).all(|pair| pair[0] < pair[1])
    }

    #[cfg(test)]
    fn spilled(&self) -> bool {
        self.0.spilled()
    }
}

impl Default for LabelSet {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for LabelSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: SmallVec<[IStr; 3]> = SmallVec::deserialize(deserializer)?;
        for window in raw.windows(2) {
            if window[0] >= window[1] {
                return Err(serde::de::Error::custom(
                    "LabelSet must be sorted by IStr order with no duplicates",
                ));
            }
        }
        Ok(Self(raw))
    }
}

impl FromIterator<IStr> for LabelSet {
    fn from_iter<T: IntoIterator<Item = IStr>>(iter: T) -> Self {
        let mut set = Self::new();
        for label in iter {
            set.insert(label);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::intern;

    fn label(name: &str) -> IStr {
        intern(name).unwrap()
    }

    #[test]
    fn insert_remove_contains_round_trip() {
        let a = label("ls.a");
        let mut set = LabelSet::new();
        assert!(set.insert(a));
        assert!(set.contains(&a));
        assert!(set.remove(&a));
        assert!(!set.contains(&a));
    }

    #[test]
    fn insert_returns_false_on_duplicate() {
        let a = label("ls.dup");
        let mut set = LabelSet::new();
        assert!(set.insert(a));
        assert!(!set.insert(a));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn iter_yields_sorted_order() {
        let a = label("ls.sorted.a");
        let b = label("ls.sorted.b");
        let set = LabelSet::from_iter([b, a]);
        assert_eq!(set.iter().copied().collect::<Vec<_>>(), vec![a, b]);
    }

    #[test]
    fn set_with_three_inline_does_not_spill() {
        let set = LabelSet::from_iter(["ls.i.1", "ls.i.2", "ls.i.3"].map(label));
        assert_eq!(set.len(), 3);
        assert!(!set.spilled());
    }

    #[test]
    fn set_with_four_or_more_spills_to_heap() {
        let set = LabelSet::from_iter(["ls.s.1", "ls.s.2", "ls.s.3", "ls.s.4"].map(label));
        assert_eq!(set.len(), 4);
        assert!(set.spilled());
    }

    #[test]
    fn from_iter_dedups_and_sorts() {
        let a = label("ls.dedup.a");
        let b = label("ls.dedup.b");
        let set = LabelSet::from_iter([b, a, b]);
        assert_eq!(set.iter().copied().collect::<Vec<_>>(), vec![a, b]);
    }

    #[test]
    fn eq_independent_of_insertion_order() {
        let a = label("ls.eq.a");
        let b = label("ls.eq.b");
        assert_eq!(LabelSet::from_iter([a, b]), LabelSet::from_iter([b, a]));
    }

    #[test]
    fn deserialize_round_trips_sorted_set() {
        let a = label("ls.de.a");
        let b = label("ls.de.b");
        let set = LabelSet::from_iter([a, b]);
        let bytes = postcard::to_allocvec(&set).unwrap();
        let round: LabelSet = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(round, set);
    }

    #[test]
    fn deserialize_rejects_unsorted_payload() {
        let a = label("ls.de.bad.a");
        let b = label("ls.de.bad.b");
        let bytes = postcard::to_allocvec::<SmallVec<[IStr; 3]>>(&{
            let mut v = SmallVec::<[IStr; 3]>::new();
            v.push(b);
            v.push(a);
            v
        })
        .unwrap();
        let result: Result<LabelSet, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_rejects_duplicate_payload() {
        let a = label("ls.de.dup.a");
        let bytes = postcard::to_allocvec::<SmallVec<[IStr; 3]>>(&{
            let mut v = SmallVec::<[IStr; 3]>::new();
            v.push(a);
            v.push(a);
            v
        })
        .unwrap();
        let result: Result<LabelSet, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn empty_single_and_large_sets() {
        assert!(LabelSet::new().is_empty());
        assert_eq!(LabelSet::single(label("ls.one")).len(), 1);
        let large = LabelSet::from_iter((0..100).map(|idx| {
            let name = format!("ls.large.{idx}");
            intern(&name).unwrap()
        }));
        assert_eq!(large.len(), 100);
        assert!(large.sorted_deduped_invariant_holds());
    }

    proptest! {
        #[test]
        fn random_inserts_are_sorted_and_deduped(raw in proptest::collection::vec(0_u8..64, 1..128)) {
            let mut set = LabelSet::new();
            let mut expected = std::collections::BTreeSet::new();
            for value in raw {
                let name = format!("ls.prop.{value}");
                let label = intern(&name).unwrap();
                let inserted = set.insert(label);
                prop_assert_eq!(inserted, expected.insert(label));
                prop_assert!(set.sorted_deduped_invariant_holds());
                prop_assert_eq!(set.len(), expected.len());
            }
        }
    }
}
