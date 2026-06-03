//! Property maps per spec 02 section 5.2.
//!
//! Keys are ordered in memory lexicographically by [`IStr`] (through the inner
//! `CompactString`) for fast binary-search lookups. Serialize canonicalizes
//! (sorts) the keys before emitting — a no-op for the common case (construction
//! keeps them sorted) but load-bearing because the `Standard`/`Compact` variants
//! are public and can be built non-canonically. Deserialize then *validates* the
//! canonical invariant — strictly-ascending, no-duplicate keys — rejecting a
//! non-canonical payload as malformed rather than re-sorting it.
//! Compact maps are closed-shape views over a fixed key set; inserting an
//! unknown key widens them to the standard open representation.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::{CoreError, CoreResult, IStr, Value};

const MAX_PROPERTY_COUNT: usize = u32::MAX as usize;

/// Property storage for open and closed graph values.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyMap {
    /// Open graph representation: sorted key/value pairs.
    Standard(SmallVec<[(IStr, Value); 6]>),
    /// Closed graph representation: fixed sorted keys with positional values.
    Compact {
        /// Sorted key set defined by a schema type.
        keys: Arc<[IStr]>,
        /// Positional values aligned with `keys`; `None` means absent.
        values: SmallVec<[Option<Value>; 6]>,
    },
}

impl PropertyMap {
    /// Construct an empty standard property map.
    #[must_use]
    pub fn new() -> Self {
        Self::Standard(SmallVec::new())
    }

    /// Construct a standard property map from pairs, sorting by `IStr` order.
    ///
    /// Later duplicate keys overwrite earlier keys.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::ConstructedValueTooLarge`] if the final distinct
    /// key count exceeds the implementation-defined cardinality cap.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (IStr, Value)>) -> CoreResult<Self> {
        let mut entries = pairs.into_iter().collect::<SmallVec<[(IStr, Value); 6]>>();
        // `sort_by` is stable: equal keys keep source order, so the collapse
        // loop below preserves the documented "later duplicate wins" contract.
        entries.sort_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));

        let mut deduped = SmallVec::new();
        for (key, value) in entries {
            if let Some((last_key, last_value)) = deduped.last_mut()
                && last_key == &key
            {
                *last_value = value;
                continue;
            }
            deduped.push((key, value));
        }
        ensure_within_cap(deduped.len())?;
        Ok(Self::Standard(deduped))
    }

    /// Construct a compact property map from a fixed schema key set.
    ///
    /// Keys are sorted with their corresponding value slots. Duplicate keys
    /// keep the last value.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::ConstructedValueTooLarge`] if key count exceeds the
    /// implementation-defined cardinality cap.
    pub fn compact(
        keys: impl IntoIterator<Item = IStr>,
        values: impl IntoIterator<Item = Option<Value>>,
    ) -> CoreResult<Self> {
        let keys: Vec<IStr> = keys.into_iter().collect();
        let values: Vec<Option<Value>> = values.into_iter().collect();
        if keys.len() != values.len() {
            return Err(CoreError::CompactKeyValueLengthMismatch {
                keys: keys.len(),
                values: values.len(),
            });
        }
        let mut slots: Vec<(IStr, Option<Value>)> = keys.into_iter().zip(values).collect();
        ensure_within_cap(slots.len())?;
        slots.sort_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
        slots.dedup_by(|(lhs_key, lhs_value), (rhs_key, rhs_value)| {
            if lhs_key == rhs_key {
                *lhs_value = rhs_value.take();
                true
            } else {
                false
            }
        });
        let (keys, values): (Vec<_>, SmallVec<_>) = slots.into_iter().unzip();
        Ok(Self::Compact {
            keys: Arc::from(keys),
            values,
        })
    }

    /// Return the value for `key`, if present.
    #[must_use]
    pub fn get(&self, key: &IStr) -> Option<&Value> {
        match self {
            Self::Standard(entries) => entries
                .binary_search_by(|(entry_key, _)| entry_key.cmp(key))
                .ok()
                .map(|idx| &entries[idx].1),
            Self::Compact { keys, values } => keys
                .binary_search(key)
                .ok()
                .and_then(|idx| values.get(idx))
                .and_then(Option::as_ref),
        }
    }

    /// Set `key` to `value`.
    ///
    /// Unknown-key writes against a compact map widen it to standard form and
    /// drop absent compact slots because standard maps only store present
    /// key/value pairs.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::ConstructedValueTooLarge`] if inserting a distinct
    /// key would exceed the implementation-defined cardinality cap.
    pub fn set(&mut self, key: IStr, value: Value) -> CoreResult<()> {
        match self {
            Self::Standard(entries) => set_standard(entries, key, value),
            Self::Compact { keys, values } => match keys.binary_search(&key) {
                Ok(idx) => {
                    values[idx] = Some(value);
                    Ok(())
                }
                Err(_) => {
                    let mut entries = compact_to_standard(keys, values);
                    set_standard(&mut entries, key, value)?;
                    *self = Self::Standard(entries);
                    Ok(())
                }
            },
        }
    }

    /// Remove a present property.
    pub fn remove(&mut self, key: &IStr) -> Option<Value> {
        match self {
            Self::Standard(entries) => entries
                .binary_search_by(|(entry_key, _)| entry_key.cmp(key))
                .ok()
                .map(|idx| entries.remove(idx).1),
            Self::Compact { keys, values } => keys
                .binary_search(key)
                .ok()
                .and_then(|idx| values.get_mut(idx))
                .and_then(Option::take),
        }
    }

    /// Number of present properties.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Standard(entries) => entries.len(),
            Self::Compact { values, .. } => values.iter().filter(|value| value.is_some()).count(),
        }
    }

    /// Return true if no properties are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate present key/value pairs in sorted-key order.
    ///
    /// Returns a concrete [`PropertyMapIter`] (not a boxed trait object): this
    /// is exercised per-label per-node on the commit path and per-write during
    /// validation, so the allocation a `Box<dyn Iterator>` would cost on every
    /// call is removed here.
    #[must_use]
    pub fn iter(&self) -> PropertyMapIter<'_> {
        match self {
            Self::Standard(entries) => PropertyMapIter::Standard(entries.iter()),
            Self::Compact { keys, values } => PropertyMapIter::Compact {
                keys: keys.iter(),
                values: values.iter(),
            },
        }
    }

    /// Iterate present property keys in sorted-key order.
    #[must_use]
    pub fn keys(&self) -> PropertyMapKeys<'_> {
        PropertyMapKeys(self.iter())
    }

    /// Iterate present property values in sorted-key order.
    #[must_use]
    pub fn values(&self) -> PropertyMapValues<'_> {
        PropertyMapValues(self.iter())
    }

    /// Return true if `key` has a present value.
    #[must_use]
    pub fn contains_key(&self, key: &IStr) -> bool {
        self.get(key).is_some()
    }

    #[cfg(test)]
    fn sorted_invariant_holds(&self) -> bool {
        match self {
            Self::Standard(entries) => entries.windows(2).all(|pair| pair[0].0 < pair[1].0),
            Self::Compact { keys, values } => {
                keys.len() == values.len() && keys.windows(2).all(|pair| pair[0] < pair[1])
            }
        }
    }
}

impl Default for PropertyMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Borrowing iterator over a [`PropertyMap`]'s present key/value pairs.
///
/// Concrete (non-boxed) so the hot commit/validate paths pay no per-call
/// allocation. Yields pairs in sorted-key order for both representations.
#[derive(Debug)]
pub enum PropertyMapIter<'a> {
    /// Iterator over a [`PropertyMap::Standard`] entry slice.
    Standard(std::slice::Iter<'a, (IStr, Value)>),
    /// Iterator over a [`PropertyMap::Compact`] key/value slot pair.
    Compact {
        /// Sorted key slots.
        keys: std::slice::Iter<'a, IStr>,
        /// Positional value slots aligned with `keys`; `None` means absent.
        values: std::slice::Iter<'a, Option<Value>>,
    },
}

impl<'a> Iterator for PropertyMapIter<'a> {
    type Item = (&'a IStr, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Standard(entries) => entries.next().map(|(key, value)| (key, value)),
            Self::Compact { keys, values } => loop {
                // Compact maps store absent slots inline; skip them so the
                // observable sequence matches Standard's "present only" contract.
                let value = values.next()?;
                let key = keys.next()?;
                if let Some(value) = value.as_ref() {
                    return Some((key, value));
                }
            },
        }
    }
}

/// Borrowing iterator over a [`PropertyMap`]'s present keys in sorted-key order.
#[derive(Debug)]
pub struct PropertyMapKeys<'a>(PropertyMapIter<'a>);

impl<'a> Iterator for PropertyMapKeys<'a> {
    type Item = &'a IStr;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(key, _)| key)
    }
}

/// Borrowing iterator over a [`PropertyMap`]'s present values in sorted-key order.
#[derive(Debug)]
pub struct PropertyMapValues<'a>(PropertyMapIter<'a>);

impl<'a> Iterator for PropertyMapValues<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(_, value)| value)
    }
}

#[derive(Deserialize, Serialize)]
enum PropertyMapWire {
    Standard(SmallVec<[(IStr, Value); 6]>),
    Compact {
        keys: Arc<[IStr]>,
        values: SmallVec<[Option<Value>; 6]>,
    },
}

impl Serialize for PropertyMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Canonicalize on serialize. Construction through `set_standard` /
        // `compact` / `from_pairs` already keeps keys in lexicographic `IStr`
        // order, so this sort is a no-op (byte-identical) for those values. But
        // `Standard` / `Compact` are PUBLIC variants, so a caller can build a
        // non-canonical map directly; canonicalizing here guarantees the wire
        // is always canonical and round-trips through the strict (validate,
        // no-resort) deserializer below. The deserializer rejects a
        // non-canonical payload rather than silently re-sorting it.
        match self {
            Self::Standard(entries) => {
                let mut entries = entries.clone();
                entries.sort_by(|(lhs, _), (rhs, _)| lhs.as_str().cmp(rhs.as_str()));
                PropertyMapWire::Standard(entries).serialize(serializer)
            }
            Self::Compact { keys, values } => {
                let mut pairs: Vec<(IStr, Option<Value>)> =
                    keys.iter().cloned().zip(values.iter().cloned()).collect();
                pairs.sort_by(|(lhs, _), (rhs, _)| lhs.as_str().cmp(rhs.as_str()));
                let (keys, values): (Vec<_>, SmallVec<_>) = pairs.into_iter().unzip();
                PropertyMapWire::Compact {
                    keys: Arc::from(keys),
                    values,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for PropertyMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // The wire is canonical (lexicographic, dedup'd) by construction, so
        // the decoder validates that invariant rather than re-sorting:
        // strictly-ascending keys (which also rejects duplicates) and the
        // Compact key/value length match. A non-canonical or duplicate-keyed
        // payload is rejected as malformed.
        let wire = PropertyMapWire::deserialize(deserializer)?;
        match wire {
            PropertyMapWire::Standard(entries) => {
                for window in entries.windows(2) {
                    if window[0].0 >= window[1].0 {
                        return Err(serde::de::Error::custom(
                            "PropertyMap::Standard entries must be sorted by IStr order with no duplicate keys",
                        ));
                    }
                }
                Ok(Self::Standard(entries))
            }
            PropertyMapWire::Compact { keys, values } => {
                if keys.len() != values.len() {
                    return Err(serde::de::Error::custom(format!(
                        "PropertyMap::Compact key/value length mismatch: {} keys, {} values",
                        keys.len(),
                        values.len(),
                    )));
                }
                for window in keys.windows(2) {
                    if window[0] >= window[1] {
                        return Err(serde::de::Error::custom(
                            "PropertyMap::Compact keys must be sorted by IStr order with no duplicates",
                        ));
                    }
                }
                Ok(Self::Compact { keys, values })
            }
        }
    }
}

fn ensure_within_cap(count: usize) -> CoreResult<()> {
    if count > MAX_PROPERTY_COUNT {
        Err(CoreError::ConstructedValueTooLarge {
            got: count,
            max: u32::MAX,
        })
    } else {
        Ok(())
    }
}

fn set_standard(
    entries: &mut SmallVec<[(IStr, Value); 6]>,
    key: IStr,
    value: Value,
) -> CoreResult<()> {
    match entries.binary_search_by(|(entry_key, _)| entry_key.cmp(&key)) {
        Ok(idx) => {
            entries[idx].1 = value;
            Ok(())
        }
        Err(idx) => {
            ensure_within_cap(entries.len().saturating_add(1))?;
            entries.insert(idx, (key, value));
            Ok(())
        }
    }
}

fn compact_to_standard(
    keys: &Arc<[IStr]>,
    values: &SmallVec<[Option<Value>; 6]>,
) -> SmallVec<[(IStr, Value); 6]> {
    keys.iter()
        .cloned()
        .zip(values.iter())
        .filter_map(|(key, value)| value.clone().map(|value| (key, value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::intern;

    fn key(name: &str) -> IStr {
        intern(name).unwrap()
    }

    fn int(value: i64) -> Value {
        Value::Int(value)
    }

    #[test]
    fn standard_get_set_remove_round_trip() {
        let mut map = PropertyMap::new();
        let name = key("pm.name");
        map.set(name.clone(), int(1)).unwrap();
        assert_eq!(map.get(&name), Some(&int(1)));
        assert_eq!(map.remove(&name), Some(int(1)));
        assert!(map.get(&name).is_none());
    }

    #[test]
    fn standard_keys_remain_sorted_after_inserts() {
        let mut map = PropertyMap::new();
        for name in ["pm.c", "pm.a", "pm.b"] {
            map.set(key(name), Value::String(key(name))).unwrap();
        }
        assert!(map.sorted_invariant_holds());
    }

    #[test]
    fn from_pairs_sorts_and_keeps_last_duplicate_value() {
        let a = key("pm.from_pairs.a");
        let b = key("pm.from_pairs.b");
        let map = PropertyMap::from_pairs([
            (b.clone(), int(10)),
            (a.clone(), int(1)),
            (b.clone(), int(20)),
            (a.clone(), int(2)),
        ])
        .unwrap();

        assert_eq!(map.get(&a), Some(&int(2)));
        assert_eq!(map.get(&b), Some(&int(20)));
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            vec![a.clone(), b.clone()]
        );
        assert!(map.sorted_invariant_holds());
    }

    #[test]
    fn compact_get_by_known_key() {
        let a = key("pm.compact.a");
        let b = key("pm.compact.b");
        let map = PropertyMap::compact([a.clone(), b.clone()], [Some(int(1)), None]).unwrap();
        assert_eq!(map.get(&a), Some(&int(1)));
        assert_eq!(map.get(&b), None);
    }

    #[test]
    fn compact_widens_on_unknown_key_insert() {
        let a = key("pm.widen.a");
        let b = key("pm.widen.b");
        let c = key("pm.widen.c");
        let mut map =
            PropertyMap::compact([a.clone(), b.clone()], [Some(int(1)), Some(int(2))]).unwrap();
        map.set(c.clone(), int(3)).unwrap();
        assert!(matches!(map, PropertyMap::Standard(_)));
        assert_eq!(map.get(&a), Some(&int(1)));
        assert_eq!(map.get(&b), Some(&int(2)));
        assert_eq!(map.get(&c), Some(&int(3)));
    }

    #[test]
    fn widening_preserves_optional_values() {
        let a = key("pm.none.a");
        let b = key("pm.none.b");
        let c = key("pm.none.c");
        let mut map = PropertyMap::compact([a.clone(), b.clone()], [None, Some(int(2))]).unwrap();
        map.set(c, int(3)).unwrap();
        assert_eq!(map.get(&a), None);
        assert_eq!(map.get(&b), Some(&int(2)));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn iter_yields_keys_in_sorted_order() {
        let a = key("pm.iter.a");
        let b = key("pm.iter.b");
        let standard = PropertyMap::from_pairs([(b.clone(), int(2)), (a.clone(), int(1))]).unwrap();
        assert_eq!(
            standard.keys().cloned().collect::<Vec<_>>(),
            vec![a.clone(), b.clone()]
        );

        let compact =
            PropertyMap::compact([b.clone(), a.clone()], [Some(int(2)), Some(int(1))]).unwrap();
        assert_eq!(compact.keys().cloned().collect::<Vec<_>>(), vec![a, b]);
    }

    #[test]
    fn len_is_empty_consistency() {
        let mut map = PropertyMap::new();
        assert!(map.is_empty());
        map.set(key("pm.len"), int(1)).unwrap();
        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
    }

    #[test]
    fn compact_rejects_mismatched_key_value_lengths() {
        let a = key("pm.mismatch.a");
        let b = key("pm.mismatch.b");
        let err = PropertyMap::compact([a.clone(), b], [Some(int(1))]).unwrap_err();
        assert!(matches!(
            err,
            CoreError::CompactKeyValueLengthMismatch { keys: 2, values: 1 }
        ));
        let err = PropertyMap::compact([a], [Some(int(1)), Some(int(2))]).unwrap_err();
        assert!(matches!(
            err,
            CoreError::CompactKeyValueLengthMismatch { keys: 1, values: 2 }
        ));
    }

    #[test]
    fn deserialize_round_trips_canonical_standard_keys() {
        // Canonical (lexicographically sorted) Standard wire round-trips and
        // preserves the sorted invariant. `IStr` Ord is lexicographic, so
        // "apple" sorts before "zebra".
        let b = key("pm.de.std.zebra");
        let a = key("pm.de.std.apple");
        let mut entries: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        entries.push((a.clone(), int(1)));
        entries.push((b.clone(), int(2)));
        let wire = PropertyMapWire::Standard(entries);
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: PropertyMap = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(result.get(&a), Some(&int(1)));
        assert_eq!(result.get(&b), Some(&int(2)));
        assert!(result.sorted_invariant_holds());
    }

    #[test]
    fn deserialize_rejects_non_canonical_standard_keys() {
        // A Standard wire whose keys are NOT in ascending IStr order is now
        // rejected as malformed (deserialize validates, no longer resorts).
        let zebra = key("pm.de.std.noncanon.zebra");
        let apple = key("pm.de.std.noncanon.apple");
        let mut entries: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        entries.push((zebra, int(2)));
        entries.push((apple, int(1)));
        let wire = PropertyMapWire::Standard(entries);
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: Result<PropertyMap, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_rejects_duplicate_standard_keys() {
        let a = key("pm.de.std.dup");
        let mut entries: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        entries.push((a.clone(), int(1)));
        entries.push((a, int(2)));
        let wire = PropertyMapWire::Standard(entries);
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: Result<PropertyMap, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_rejects_compact_length_mismatch() {
        let a = key("pm.de.cmp.a");
        let b = key("pm.de.cmp.b");
        let bad = PropertyMapWire::Compact {
            keys: Arc::from([a, b]),
            values: SmallVec::from_vec(vec![Some(int(1))]),
        };
        let bad_bytes = postcard::to_allocvec(&bad_wire_map(bad)).unwrap();
        let result: Result<PropertyMap, _> = postcard::from_bytes(&bad_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_round_trips_canonical_compact_keys_and_values() {
        // Canonical (sorted-key) Compact wire round-trips with values aligned.
        let b = key("pm.de.cmpsort.zebra");
        let a = key("pm.de.cmpsort.apple");
        let wire = PropertyMapWire::Compact {
            keys: Arc::from([a.clone(), b.clone()]),
            values: SmallVec::from_vec(vec![Some(int(1)), Some(int(2))]),
        };
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: PropertyMap = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(result.get(&a), Some(&int(1)));
        assert_eq!(result.get(&b), Some(&int(2)));
        assert!(result.sorted_invariant_holds());
    }

    #[test]
    fn deserialize_rejects_non_canonical_compact_keys() {
        // A Compact wire whose key list is NOT ascending is rejected as
        // malformed (deserialize validates the canonical invariant).
        let zebra = key("pm.de.cmpsort.noncanon.zebra");
        let apple = key("pm.de.cmpsort.noncanon.apple");
        let wire = PropertyMapWire::Compact {
            keys: Arc::from([zebra, apple]),
            values: SmallVec::from_vec(vec![Some(int(2)), Some(int(1))]),
        };
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: Result<PropertyMap, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_rejects_duplicate_compact_keys() {
        let a = key("pm.de.cmpdup.a");
        let bad = PropertyMapWire::Compact {
            keys: Arc::from([a.clone(), a]),
            values: SmallVec::from_vec(vec![Some(int(1)), Some(int(2))]),
        };
        let bytes = postcard::to_allocvec(&bad_wire_map(bad)).unwrap();
        let result: Result<PropertyMap, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    fn bad_wire_map(wire: PropertyMapWire) -> PropertyMapWireSer {
        match wire {
            PropertyMapWire::Standard(entries) => PropertyMapWireSer::Standard(entries),
            PropertyMapWire::Compact { keys, values } => {
                PropertyMapWireSer::Compact { keys, values }
            }
        }
    }

    #[derive(serde::Serialize)]
    enum PropertyMapWireSer {
        Standard(SmallVec<[(IStr, Value); 6]>),
        Compact {
            keys: Arc<[IStr]>,
            values: SmallVec<[Option<Value>; 6]>,
        },
    }

    #[test]
    fn serialize_emits_canonical_wire_bytes() {
        // Wire-invariance proof: a PropertyMap built from out-of-order pairs
        // (`from_pairs` sorts on construction) serializes to the exact canonical
        // (sorted) wire bytes — the wire is byte-identical to the pre-removal
        // format because the in-memory order is already lexicographic.
        let zebra = key("pm.wire.zebra");
        let apple = key("pm.wire.apple");
        let mango = key("pm.wire.mango");
        let map = PropertyMap::from_pairs([
            (zebra.clone(), int(3)),
            (apple.clone(), int(1)),
            (mango.clone(), int(2)),
        ])
        .unwrap();
        let map_bytes = postcard::to_allocvec(&map).unwrap();

        // The canonical wire is the same keys in sorted order.
        let mut canonical: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        canonical.push((apple, int(1)));
        canonical.push((mango, int(2)));
        canonical.push((zebra, int(3)));
        let canonical_bytes =
            postcard::to_allocvec(&bad_wire_map(PropertyMapWire::Standard(canonical))).unwrap();

        assert_eq!(
            map_bytes, canonical_bytes,
            "serialize must emit canonical lexicographic bytes"
        );

        // And the bytes round-trip back to an equal map.
        let round: PropertyMap = postcard::from_bytes(&map_bytes).unwrap();
        assert_eq!(round, map);
    }

    #[test]
    fn serialize_canonicalizes_non_canonical_standard_then_round_trips() {
        // `PropertyMap::Standard` is a PUBLIC variant, so a caller can build a
        // non-canonical (out-of-order) map without going through `set_standard`
        // / `from_pairs`. Serialize canonicalizes it so the wire is always
        // canonical and round-trips through the strict deserializer — guarding
        // the public-construction path against the validate-no-resort decoder.
        let zebra = key("pm.noncanon.zebra");
        let apple = key("pm.noncanon.apple");
        let mut entries: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        entries.push((zebra.clone(), int(2)));
        entries.push((apple.clone(), int(1)));
        let non_canonical = PropertyMap::Standard(entries);
        let bytes = postcard::to_allocvec(&non_canonical).unwrap();

        // Bytes are canonical (apple before zebra), so the strict decoder
        // accepts them and the value round-trips.
        let round: PropertyMap = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(round.get(&apple), Some(&int(1)));
        assert_eq!(round.get(&zebra), Some(&int(2)));

        let canonical = PropertyMap::from_pairs([(apple, int(1)), (zebra, int(2))]).unwrap();
        assert_eq!(
            bytes,
            postcard::to_allocvec(&canonical).unwrap(),
            "non-canonical construction must serialize to the same canonical bytes"
        );
    }

    #[test]
    fn serialize_independent_of_insertion_order() {
        // Two maps built from different insertion orders of the same pairs
        // serialize to byte-identical wire (canonical lexicographic order).
        let pairs = [
            (key("pm.order.gamma"), int(3)),
            (key("pm.order.alpha"), int(1)),
            (key("pm.order.beta"), int(2)),
        ];
        let forward = PropertyMap::from_pairs(pairs.iter().cloned()).unwrap();
        let reverse = PropertyMap::from_pairs(pairs.iter().rev().cloned()).unwrap();
        assert_eq!(
            postcard::to_allocvec(&forward).unwrap(),
            postcard::to_allocvec(&reverse).unwrap(),
        );
    }

    #[test]
    fn edge_cases_for_missing_and_overwrite() {
        let a = key("pm.edge.a");
        let mut map = PropertyMap::new();
        assert_eq!(map.get(&a), None);
        assert_eq!(map.remove(&a), None);
        map.set(a.clone(), int(1)).unwrap();
        map.set(a.clone(), int(2)).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.remove(&a), Some(int(2)));
        assert!(map.is_empty());
    }

    proptest! {
        #[test]
        fn standard_insert_remove_preserves_sorted_invariant(ops in proptest::collection::vec((0_u8..32, any::<bool>()), 1..128)) {
            let mut map = PropertyMap::new();
            let mut expected = std::collections::BTreeSet::new();
            for (raw, insert) in ops {
                let name = format!("pm.prop.{raw}");
                let key = intern(&name).unwrap();
                if insert {
                    map.set(key.clone(), Value::Uint(u64::from(raw))).unwrap();
                    expected.insert(key);
                } else {
                    map.remove(&key);
                    expected.remove(&key);
                }
                prop_assert!(map.sorted_invariant_holds());
                prop_assert_eq!(map.len(), expected.len());
            }
        }

        /// CORE-16: a Compact map seeded with a fixed key set, then driven
        /// through sets/removes (including unknown-key widening), must stay
        /// observationally equal to a BTreeMap oracle where `None == absent`.
        #[test]
        fn compact_widening_matches_reference_map(
            // Slot presence for the four schema keys at construction.
            seed in proptest::collection::vec(any::<bool>(), 4),
            // (key index 0..6, present?, value) operations; keys 4 and 5 are
            // unknown to the seed key set, so they exercise the widening path.
            ops in proptest::collection::vec((0_usize..6, any::<bool>(), 0_u64..16), 0..64),
        ) {
            let pid = std::process::id();
            let schema_keys: Vec<IStr> = (0..4)
                .map(|i| intern(&format!("pm.core16.{pid}.k{i}")).unwrap())
                .collect();
            let all_keys: Vec<IStr> = (0..6)
                .map(|i| intern(&format!("pm.core16.{pid}.k{i}")).unwrap())
                .collect();

            let mut oracle = std::collections::BTreeMap::<IStr, Value>::new();
            let seed_values: Vec<Option<Value>> = schema_keys
                .iter()
                .zip(&seed)
                .map(|(k, present)| {
                    if *present {
                        oracle.insert(k.clone(), Value::Uint(0));
                        Some(Value::Uint(0))
                    } else {
                        None
                    }
                })
                .collect();

            let mut map = PropertyMap::compact(schema_keys.iter().cloned(), seed_values).unwrap();
            let is_compact = matches!(map, PropertyMap::Compact { .. });
            prop_assert!(is_compact);

            for (idx, present, raw) in ops {
                let key = all_keys[idx].clone();
                if present {
                    let value = Value::Uint(raw);
                    map.set(key.clone(), value.clone()).unwrap();
                    oracle.insert(key, value);
                } else {
                    let removed = map.remove(&key);
                    let expected = oracle.remove(&key);
                    prop_assert_eq!(removed, expected);
                }

                // Observational equality: get() agrees, including None == absent.
                for k in &all_keys {
                    prop_assert_eq!(map.get(k), oracle.get(k));
                }
                prop_assert_eq!(map.len(), oracle.len());
                prop_assert!(map.sorted_invariant_holds());

                // iter() yields exactly the oracle's present pairs in key order.
                let observed: Vec<(IStr, Value)> =
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let expected: Vec<(IStr, Value)> =
                    oracle.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                // Both are key-handle-sorted (BTreeMap by IStr Ord, map by invariant).
                prop_assert_eq!(observed, expected);
            }
        }
    }
}
