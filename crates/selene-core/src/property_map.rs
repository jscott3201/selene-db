//! Property maps per spec 02 section 5.2.
//!
//! Keys are ordered in memory by [`IStr`] interner-key order for fast lookups.
//! The wire format is different: maps serialize keys in canonical
//! lexicographic order by [`IStr::as_str`], and deserialize by re-sorting into
//! the receiver's local interner-key order before validating duplicate keys.
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

    /// Construct an empty standard property map with capacity.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::ConstructedValueTooLarge`] if `capacity` exceeds
    /// the implementation-defined property cardinality cap.
    pub fn with_capacity(capacity: usize) -> CoreResult<Self> {
        ensure_within_cap(capacity)?;
        Ok(Self::Standard(SmallVec::with_capacity(capacity)))
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
        let mut map = Self::new();
        for (key, value) in pairs {
            map.set(key, value)?;
        }
        Ok(map)
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
        slots.sort_by_key(|(key, _)| *key);
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
                .binary_search_by_key(key, |(entry_key, _)| *entry_key)
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
                .binary_search_by_key(key, |(entry_key, _)| *entry_key)
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
    pub fn iter(&self) -> Box<dyn Iterator<Item = (&IStr, &Value)> + '_> {
        match self {
            Self::Standard(entries) => Box::new(entries.iter().map(|(key, value)| (key, value))),
            Self::Compact { keys, values } => Box::new(
                keys.iter()
                    .zip(values.iter())
                    .filter_map(|(key, value)| value.as_ref().map(|value| (key, value))),
            ),
        }
    }

    /// Iterate present property keys in sorted-key order.
    pub fn keys(&self) -> Box<dyn Iterator<Item = &IStr> + '_> {
        Box::new(self.iter().map(|(key, _)| key))
    }

    /// Iterate present property values in sorted-key order.
    pub fn values(&self) -> Box<dyn Iterator<Item = &Value> + '_> {
        Box::new(self.iter().map(|(_, value)| value))
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
        match self {
            Self::Standard(entries) => {
                let mut entries = entries.clone();
                entries.sort_by(|(lhs, _), (rhs, _)| lhs.as_str().cmp(rhs.as_str()));
                PropertyMapWire::Standard(entries).serialize(serializer)
            }
            Self::Compact { keys, values } => {
                let mut pairs: Vec<(IStr, Option<Value>)> =
                    keys.iter().copied().zip(values.iter().cloned()).collect();
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
        let wire = PropertyMapWire::deserialize(deserializer)?;
        match wire {
            PropertyMapWire::Standard(mut entries) => {
                entries.sort_unstable_by_key(|(key, _)| *key);
                for window in entries.windows(2) {
                    if window[0].0 >= window[1].0 {
                        return Err(serde::de::Error::custom(
                            "PropertyMap::Standard entries have duplicate keys",
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
                let mut pairs: Vec<(IStr, Option<Value>)> =
                    keys.iter().copied().zip(values).collect();
                pairs.sort_unstable_by_key(|(key, _)| *key);
                for window in pairs.windows(2) {
                    if window[0].0 >= window[1].0 {
                        return Err(serde::de::Error::custom(
                            "PropertyMap::Compact keys have duplicates",
                        ));
                    }
                }
                let (keys, values): (Vec<_>, SmallVec<_>) = pairs.into_iter().unzip();
                Ok(Self::Compact {
                    keys: Arc::from(keys),
                    values,
                })
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
    match entries.binary_search_by_key(&key, |(entry_key, _)| *entry_key) {
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
        .copied()
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
        map.set(name, int(1)).unwrap();
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
    fn compact_get_by_known_key() {
        let a = key("pm.compact.a");
        let b = key("pm.compact.b");
        let map = PropertyMap::compact([a, b], [Some(int(1)), None]).unwrap();
        assert_eq!(map.get(&a), Some(&int(1)));
        assert_eq!(map.get(&b), None);
    }

    #[test]
    fn compact_widens_on_unknown_key_insert() {
        let a = key("pm.widen.a");
        let b = key("pm.widen.b");
        let c = key("pm.widen.c");
        let mut map = PropertyMap::compact([a, b], [Some(int(1)), Some(int(2))]).unwrap();
        map.set(c, int(3)).unwrap();
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
        let mut map = PropertyMap::compact([a, b], [None, Some(int(2))]).unwrap();
        map.set(c, int(3)).unwrap();
        assert_eq!(map.get(&a), None);
        assert_eq!(map.get(&b), Some(&int(2)));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn iter_yields_keys_in_sorted_order() {
        let a = key("pm.iter.a");
        let b = key("pm.iter.b");
        let standard = PropertyMap::from_pairs([(b, int(2)), (a, int(1))]).unwrap();
        assert_eq!(standard.keys().copied().collect::<Vec<_>>(), vec![a, b]);

        let compact = PropertyMap::compact([b, a], [Some(int(2)), Some(int(1))]).unwrap();
        assert_eq!(compact.keys().copied().collect::<Vec<_>>(), vec![a, b]);
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
        let err = PropertyMap::compact([a, b], [Some(int(1))]).unwrap_err();
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
    fn deserialize_resorts_standard_keys_by_receiver_handle() {
        let b = key("pm.de.std.zebra");
        let a = key("pm.de.std.apple");
        let mut entries: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        entries.push((a, int(1)));
        entries.push((b, int(2)));
        let wire = PropertyMapWire::Standard(entries);
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: PropertyMap = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(result.get(&a), Some(&int(1)));
        assert_eq!(result.get(&b), Some(&int(2)));
        assert!(result.sorted_invariant_holds());
    }

    #[test]
    fn deserialize_rejects_duplicate_standard_keys() {
        let a = key("pm.de.std.dup");
        let mut entries: SmallVec<[(IStr, Value); 6]> = SmallVec::new();
        entries.push((a, int(1)));
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
    fn deserialize_resorts_compact_keys_and_values_by_receiver_handle() {
        let b = key("pm.de.cmpsort.zebra");
        let a = key("pm.de.cmpsort.apple");
        let wire = PropertyMapWire::Compact {
            keys: Arc::from([a, b]),
            values: SmallVec::from_vec(vec![Some(int(1)), Some(int(2))]),
        };
        let bytes = postcard::to_allocvec(&bad_wire_map(wire)).unwrap();
        let result: PropertyMap = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(result.get(&a), Some(&int(1)));
        assert_eq!(result.get(&b), Some(&int(2)));
        assert!(result.sorted_invariant_holds());
    }

    #[test]
    fn deserialize_rejects_duplicate_compact_keys() {
        let a = key("pm.de.cmpdup.a");
        let bad = PropertyMapWire::Compact {
            keys: Arc::from([a, a]),
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
    fn edge_cases_for_missing_and_overwrite() {
        let a = key("pm.edge.a");
        let mut map = PropertyMap::new();
        assert_eq!(map.get(&a), None);
        assert_eq!(map.remove(&a), None);
        map.set(a, int(1)).unwrap();
        map.set(a, int(2)).unwrap();
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
                    map.set(key, Value::Uint(u64::from(raw))).unwrap();
                    expected.insert(key);
                } else {
                    map.remove(&key);
                    expected.remove(&key);
                }
                prop_assert!(map.sorted_invariant_holds());
                prop_assert_eq!(map.len(), expected.len());
            }
        }
    }
}
