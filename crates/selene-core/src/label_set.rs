//! Sorted label sets per spec 02 section 5.3.
//!
//! Labels are ordered in memory lexicographically by [`DbString`]. On the wire,
//! labels serialize in that same canonical lexicographic order by
//! [`DbString::as_str`] and deserialize by *validating* that canonical order — a
//! non-ascending or duplicate payload is rejected as malformed (the in-memory
//! order is already lexicographic, so no re-sort is needed). The inline capacity
//! of 3 matches the common
//! node-label case; larger sets spill cleanly to the heap. Edges semantically
//! carry exactly one label, but that constraint is enforced by `selene-graph`,
//! not by this plain set type.

use std::error::Error;
use std::fmt;

use rkyv::{
    Archive, Archived, Deserialize as RkyvDeserialize, Place, Serialize as RkyvSerialize,
    rancor::{Fallible, Source},
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::DbString;

/// Sorted set of graph labels.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LabelSet(SmallVec<[DbString; 3]>);

#[derive(Debug)]
struct InvalidArchivedLabelSet;

impl fmt::Display for InvalidArchivedLabelSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("archived LabelSet must be sorted by DbString order with no duplicates")
    }
}

impl Error for InvalidArchivedLabelSet {}

impl LabelSet {
    /// Construct an empty label set.
    #[must_use]
    pub fn new() -> Self {
        Self(SmallVec::new())
    }

    /// Construct a label set from an iterator, sorting and deduplicating.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn from_iter(labels: impl IntoIterator<Item = DbString>) -> Self {
        labels.into_iter().collect()
    }

    /// Construct a single-label set.
    #[must_use]
    pub fn single(label: DbString) -> Self {
        let mut labels = SmallVec::new();
        labels.push(label);
        Self(labels)
    }

    /// Construct the one-label view used by edge APIs.
    #[must_use]
    pub fn edge(label: DbString) -> Self {
        Self::single(label)
    }

    /// Insert a label, returning true when it was not already present.
    pub fn insert(&mut self, label: DbString) -> bool {
        match self.0.binary_search(&label) {
            Ok(_) => false,
            Err(idx) => {
                self.0.insert(idx, label);
                true
            }
        }
    }

    /// Remove a label, returning true when it was present.
    pub fn remove(&mut self, label: &DbString) -> bool {
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
    pub fn contains(&self, label: &DbString) -> bool {
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

    /// Iterate labels in sorted `DbString` order.
    pub fn iter(&self) -> impl Iterator<Item = &DbString> {
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

impl Serialize for LabelSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // No resort: labels are kept in lexicographic `DbString` order in memory
        // (`insert` maintains the sorted-deduped invariant), and `DbString` Ord is
        // lexicographic, so this emits the canonical wire order directly —
        // byte-identical to the old `sort_by(as_str())`.
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LabelSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // The wire is canonical (strictly-ascending, dedup'd) by construction;
        // validate that invariant rather than re-sorting. A non-canonical or
        // duplicate-bearing payload is rejected as malformed.
        let raw: SmallVec<[DbString; 3]> = SmallVec::deserialize(deserializer)?;
        for window in raw.windows(2) {
            if window[0] >= window[1] {
                return Err(serde::de::Error::custom(
                    "LabelSet must be sorted by DbString order with no duplicate labels",
                ));
            }
        }
        Ok(Self(raw))
    }
}

impl Archive for LabelSet {
    type Archived = ArchivedVec<Archived<DbString>>;
    type Resolver = VecResolver;

    fn resolve(&self, resolver: Self::Resolver, out: Place<Self::Archived>) {
        ArchivedVec::resolve_from_slice(self.0.as_slice(), resolver, out);
    }
}

impl<S> RkyvSerialize<S> for LabelSet
where
    S: Fallible + Allocator + Writer + ?Sized,
    DbString: RkyvSerialize<S>,
{
    fn serialize(&self, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        // No resort: in-memory order is already canonical lexicographic order
        // (see the serde `Serialize` impl), so this archives the slice directly.
        ArchivedVec::serialize_from_slice(self.0.as_slice(), serializer)
    }
}

impl<D> RkyvDeserialize<LabelSet, D> for ArchivedVec<Archived<DbString>>
where
    D: Fallible + ?Sized,
    D::Error: Source,
    Archived<DbString>: RkyvDeserialize<DbString, D>,
{
    fn deserialize(&self, deserializer: &mut D) -> Result<LabelSet, D::Error> {
        let mut raw: SmallVec<[DbString; 3]> = SmallVec::new();
        for label in self.as_slice() {
            raw.push(label.deserialize(deserializer)?);
        }
        // Validate the canonical (strictly-ascending, dedup'd) invariant rather
        // than re-sorting; a non-canonical archive is rejected as malformed.
        for window in raw.windows(2) {
            if window[0] >= window[1] {
                rkyv::rancor::fail!(InvalidArchivedLabelSet);
            }
        }
        Ok(LabelSet(raw))
    }
}

impl FromIterator<DbString> for LabelSet {
    fn from_iter<T: IntoIterator<Item = DbString>>(iter: T) -> Self {
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
    use crate::db_string;

    fn label(name: &str) -> DbString {
        db_string(name).unwrap()
    }

    #[test]
    fn insert_remove_contains_round_trip() {
        let a = label("ls.a");
        let mut set = LabelSet::new();
        assert!(set.insert(a.clone()));
        assert!(set.contains(&a));
        assert!(set.remove(&a));
        assert!(!set.contains(&a));
    }

    #[test]
    fn insert_returns_false_on_duplicate() {
        let a = label("ls.dup");
        let mut set = LabelSet::new();
        assert!(set.insert(a.clone()));
        assert!(!set.insert(a));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn iter_yields_sorted_order() {
        let a = label("ls.sorted.a");
        let b = label("ls.sorted.b");
        let set = LabelSet::from_iter([b.clone(), a.clone()]);
        assert_eq!(set.iter().cloned().collect::<Vec<_>>(), vec![a, b]);
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
        let set = LabelSet::from_iter([b.clone(), a.clone(), b.clone()]);
        assert_eq!(set.iter().cloned().collect::<Vec<_>>(), vec![a, b]);
    }

    #[test]
    fn eq_independent_of_insertion_order() {
        let a = label("ls.eq.a");
        let b = label("ls.eq.b");
        assert_eq!(
            LabelSet::from_iter([a.clone(), b.clone()]),
            LabelSet::from_iter([b, a])
        );
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
    fn serialize_independent_of_insertion_order() {
        // Wire-invariance proof: two sets built from different insertion orders
        // of the same labels serialize to byte-identical wire (serde + rkyv),
        // confirming the dropped serialize-side resort changed no bytes.
        let labels = ["ls.wire.gamma", "ls.wire.alpha", "ls.wire.beta"];
        let forward = LabelSet::from_iter(labels.map(label));
        let mut rev = labels;
        rev.reverse();
        let reverse = LabelSet::from_iter(rev.map(label));

        assert_eq!(
            postcard::to_allocvec(&forward).unwrap(),
            postcard::to_allocvec(&reverse).unwrap(),
            "serde wire must be insertion-order-independent",
        );
        assert_eq!(
            rkyv::to_bytes::<rkyv::rancor::Error>(&forward)
                .unwrap()
                .to_vec(),
            rkyv::to_bytes::<rkyv::rancor::Error>(&reverse)
                .unwrap()
                .to_vec(),
            "rkyv archive must be insertion-order-independent",
        );
    }

    #[test]
    fn deserialize_round_trips_canonical_payload() {
        // Canonical (ascending) payload deserializes and preserves the
        // sorted-deduped invariant. "apple" < "zebra" lexicographically.
        let b = label("ls.de.canon.zebra");
        let a = label("ls.de.canon.apple");
        let bytes = postcard::to_allocvec::<SmallVec<[DbString; 3]>>(&{
            let mut v = SmallVec::<[DbString; 3]>::new();
            v.push(a.clone());
            v.push(b.clone());
            v
        })
        .unwrap();
        let result: LabelSet = postcard::from_bytes(&bytes).unwrap();
        assert!(result.contains(&a));
        assert!(result.contains(&b));
        assert!(result.sorted_deduped_invariant_holds());
    }

    #[test]
    fn deserialize_rejects_non_canonical_payload() {
        // A non-ascending payload is now rejected as malformed (validate, not
        // resort).
        let zebra = label("ls.de.noncanon.zebra");
        let apple = label("ls.de.noncanon.apple");
        let bytes = postcard::to_allocvec::<SmallVec<[DbString; 3]>>(&{
            let mut v = SmallVec::<[DbString; 3]>::new();
            v.push(zebra);
            v.push(apple);
            v
        })
        .unwrap();
        let result: Result<LabelSet, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_rejects_duplicate_payload() {
        let a = label("ls.de.dup.a");
        let bytes = postcard::to_allocvec::<SmallVec<[DbString; 3]>>(&{
            let mut v = SmallVec::<[DbString; 3]>::new();
            v.push(a.clone());
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
            db_string(&name).unwrap()
        }));
        assert_eq!(large.len(), 100);
        assert!(large.sorted_deduped_invariant_holds());
    }

    #[test]
    fn rkyv_deserialize_round_trips_sorted_set() {
        let a = label("ls.rkyv.a");
        let b = label("ls.rkyv.b");
        let set = LabelSet::from_iter([a, b]);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&set).unwrap();
        let round: LabelSet = rkyv::from_bytes::<LabelSet, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(round, set);
    }

    #[test]
    fn rkyv_deserialize_round_trips_canonical_payload() {
        // Canonical (ascending) archive deserializes and preserves the
        // sorted-deduped invariant.
        let b = label("ls.rkyv.canon.zebra");
        let a = label("ls.rkyv.canon.apple");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&vec![a.clone(), b.clone()]).unwrap();
        let result = rkyv::from_bytes::<LabelSet, rkyv::rancor::Error>(&bytes).unwrap();
        assert!(result.contains(&a));
        assert!(result.contains(&b));
        assert!(result.sorted_deduped_invariant_holds());
    }

    #[test]
    fn rkyv_deserialize_rejects_non_canonical_payload() {
        // A non-ascending archive is rejected as malformed (validate, not
        // resort).
        let zebra = label("ls.rkyv.noncanon.zebra");
        let apple = label("ls.rkyv.noncanon.apple");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&vec![zebra, apple]).unwrap();
        let result = rkyv::from_bytes::<LabelSet, rkyv::rancor::Error>(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn rkyv_deserialize_rejects_duplicate_payload() {
        let a = label("ls.rkyv.dup.a");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&vec![a.clone(), a]).unwrap();
        let result = rkyv::from_bytes::<LabelSet, rkyv::rancor::Error>(&bytes);
        assert!(result.is_err());
    }

    proptest! {
        #[test]
        fn random_inserts_are_sorted_and_deduped(raw in proptest::collection::vec(0_u8..64, 1..128)) {
            let mut set = LabelSet::new();
            let mut expected = std::collections::BTreeSet::new();
            for value in raw {
                let name = format!("ls.prop.{value}");
                let label = db_string(&name).unwrap();
                let inserted = set.insert(label.clone());
                prop_assert_eq!(inserted, expected.insert(label));
                prop_assert!(set.sorted_deduped_invariant_holds());
                prop_assert_eq!(set.len(), expected.len());
            }
        }
    }
}
