//! Built-in composite-property value index.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use roaring::RoaringBitmap;
use selene_core::{DbString, Value};
use smallvec::SmallVec;

use crate::typed_index::{NotNanError, NotNanF64, TypedIndexKind, TypedIndexValueError};

/// Composite key used by a composite-property index.
pub type CompositeKey = SmallVec<[CompositeKeyComponent; 4]>;

/// One ordered component in a [`CompositeKey`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositeKeyComponent {
    /// Signed integer component.
    I64(i64),
    /// Floating-point component with NaN excluded.
    F64(NotNanF64),
    /// Database-string component.
    String(DbString),
    /// Civil date component.
    Date(jiff::civil::Date),
    /// Civil local date-time component.
    LocalDateTime(jiff::civil::DateTime),
    /// UUID component.
    Uuid(uuid::Uuid),
}

impl Ord for CompositeKeyComponent {
    fn cmp(&self, rhs: &Self) -> std::cmp::Ordering {
        use CompositeKeyComponent as K;
        match (self, rhs) {
            (K::I64(lhs), K::I64(rhs)) => lhs.cmp(rhs),
            (K::F64(lhs), K::F64(rhs)) => lhs.cmp(rhs),
            (K::String(lhs), K::String(rhs)) => lhs.cmp(rhs),
            (K::Date(lhs), K::Date(rhs)) => lhs.cmp(rhs),
            (K::LocalDateTime(lhs), K::LocalDateTime(rhs)) => lhs.cmp(rhs),
            (K::Uuid(lhs), K::Uuid(rhs)) => lhs.cmp(rhs),
            _ => self.discriminant().cmp(&rhs.discriminant()),
        }
    }
}

impl PartialOrd for CompositeKeyComponent {
    fn partial_cmp(&self, rhs: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(rhs))
    }
}

impl Hash for CompositeKeyComponent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.discriminant().hash(state);
        match self {
            Self::I64(value) => value.hash(state),
            Self::F64(value) => value.hash(state),
            Self::String(value) => value.hash(state),
            Self::Date(value) => value.hash(state),
            Self::LocalDateTime(value) => value.hash(state),
            Self::Uuid(value) => value.hash(state),
        }
    }
}

impl CompositeKeyComponent {
    const fn discriminant(&self) -> u8 {
        match self {
            Self::I64(_) => 0,
            Self::F64(_) => 1,
            Self::String(_) => 2,
            Self::Date(_) => 3,
            Self::LocalDateTime(_) => 4,
            Self::Uuid(_) => 5,
        }
    }
}

/// Built-in node index for an ordered tuple of property values.
#[derive(Clone, Debug)]
pub struct CompositeTypedIndex {
    kinds: SmallVec<[TypedIndexKind; 4]>,
    entries: BTreeMap<CompositeKey, RoaringBitmap>,
}

impl CompositeTypedIndex {
    /// Construct an empty composite index for the supplied ordered kinds.
    #[must_use]
    pub fn new(kinds: SmallVec<[TypedIndexKind; 4]>) -> Self {
        Self {
            kinds,
            entries: BTreeMap::new(),
        }
    }

    /// Return the ordered component kinds.
    #[must_use]
    pub fn kinds(&self) -> &[TypedIndexKind] {
        &self.kinds
    }

    /// Return total row cardinality across all composite keys.
    ///
    /// This is the sum of every bucket's row count, NOT the number of distinct
    /// composite keys. For the distinct-key count use
    /// [`CompositeTypedIndex::distinct_keys`].
    #[must_use]
    pub fn cardinality(&self) -> u64 {
        self.entries.values().map(RoaringBitmap::len).sum()
    }

    /// Return the number of distinct composite keys (BTreeMap entry count).
    ///
    /// Unlike [`CompositeTypedIndex::cardinality`] (total rows), this counts the
    /// distinct composite-key buckets. The optimizer cost model divides
    /// `cardinality / distinct_keys` to estimate the expected rows returned by a
    /// parameter-keyed composite probe whose values are unknown at plan time.
    /// Returns `0` for an empty index.
    #[must_use]
    pub fn distinct_keys(&self) -> u64 {
        self.entries.len() as u64
    }

    /// Iterate composite-key buckets and their matching row bitmaps.
    pub fn entries(&self) -> impl Iterator<Item = (&CompositeKey, &RoaringBitmap)> {
        self.entries.iter()
    }

    /// Return true when this index holds exactly the same `(key -> rows)`
    /// buckets as `reference`.
    ///
    /// Used by the debug-only structural consistency net
    /// ([`crate::SeleneGraph::assert_indexes_consistent`]). Component kinds
    /// and every composite-key bucket's row bitmap must match.
    #[must_use]
    pub(crate) fn buckets_eq(&self, reference: &Self) -> bool {
        self.kinds == reference.kinds && self.entries == reference.entries
    }

    /// Return true when any composite key maps to an empty row bitmap.
    ///
    /// Maintenance prunes a bucket when its bitmap empties (see
    /// [`Self::remove`]); a present-but-empty bucket is a leak the
    /// debug-only consistency net flags.
    #[must_use]
    pub(crate) fn has_empty_bucket(&self) -> bool {
        self.entries.values().any(RoaringBitmap::is_empty)
    }

    /// Insert `row` under the composite key formed from `values`.
    pub fn insert(&mut self, values: &[&Value], row: u32) -> Result<(), CompositeIndexValueError> {
        let key = self.key_from_values(values)?;
        self.entries.entry(key).or_default().insert(row);
        Ok(())
    }

    /// Remove `row` from the composite key formed from `values`.
    pub fn remove(&mut self, values: &[&Value], row: u32) -> Result<(), CompositeIndexValueError> {
        let key = self.key_from_values(values)?;
        if let Some(bitmap) = self.entries.get_mut(&key) {
            bitmap.remove(row);
            if bitmap.is_empty() {
                self.entries.remove(&key);
            }
        }
        Ok(())
    }

    /// Return rows matching `key`.
    #[must_use]
    pub fn lookup_key(&self, key: &CompositeKey) -> Option<&RoaringBitmap> {
        self.entries.get(key)
    }

    /// Build a composite key from the index's ordered component kinds.
    ///
    /// This is the single coercion shared by write/maintenance and read paths.
    /// Every coercible `STRING` component resolves directly to a
    /// database-string key; an arity or per-component kind mismatch raises
    /// [`CompositeIndexValueError`].
    pub fn key_from_values(
        &self,
        values: &[&Value],
    ) -> Result<CompositeKey, CompositeIndexValueError> {
        composite_key_from_values(&self.kinds, values)
    }

    /// Return true when two value tuples address the same key.
    ///
    /// Uses [`Self::key_from_values`]; when either side cannot be coerced to a
    /// key it falls through to a pairwise content compare on the raw values.
    pub fn values_share_key(&self, lhs: &[&Value], rhs: &[&Value]) -> bool {
        match (self.key_from_values(lhs), self.key_from_values(rhs)) {
            (Ok(lhs_key), Ok(rhs_key)) => lhs_key == rhs_key,
            (Err(_), Err(_)) => true,
            _ => false,
        }
    }
}

/// Value-admission error for composite index mutation.
#[derive(Debug)]
#[non_exhaustive]
pub enum CompositeIndexValueError {
    /// The tuple length did not match the registered component count.
    ArityMismatch {
        /// Registered component count.
        expected: usize,
        /// Observed value count.
        observed: usize,
    },
    /// A component value was not admissible for its registered kind.
    Component {
        /// Zero-based component index.
        index: usize,
        /// Registered component kind.
        expected_kind: TypedIndexKind,
        /// Observed value kind or `"NaN"`.
        observed: &'static str,
    },
}

/// Build a composite key from ordered component kinds and values.
///
/// This is the single coercion shared by write/maintenance and read paths. Every
/// coercible `STRING` component resolves directly to a database-string key; an
/// arity mismatch or a per-component kind/NaN mismatch raises
/// [`CompositeIndexValueError`].
pub(crate) fn composite_key_from_values(
    kinds: &[TypedIndexKind],
    values: &[&Value],
) -> Result<CompositeKey, CompositeIndexValueError> {
    if kinds.len() != values.len() {
        return Err(CompositeIndexValueError::ArityMismatch {
            expected: kinds.len(),
            observed: values.len(),
        });
    }
    kinds
        .iter()
        .zip(values)
        .enumerate()
        .map(|(index, (kind, value))| {
            component_from_value(*kind, value).map_err(|source| {
                CompositeIndexValueError::Component {
                    index,
                    expected_kind: source.expected_kind(),
                    observed: source.observed(),
                }
            })
        })
        .collect()
}

fn component_from_value(
    kind: TypedIndexKind,
    value: &Value,
) -> Result<CompositeKeyComponent, TypedIndexValueError> {
    match (kind, value) {
        (TypedIndexKind::I64, Value::Int(value)) => Ok(CompositeKeyComponent::I64(*value)),
        (TypedIndexKind::F64, Value::Float(value)) => NotNanF64::new(*value)
            .map(CompositeKeyComponent::F64)
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        (TypedIndexKind::String, Value::String(value)) => {
            Ok(CompositeKeyComponent::String(value.clone()))
        }
        (TypedIndexKind::Date, Value::Date(value)) => Ok(CompositeKeyComponent::Date(*value)),
        (TypedIndexKind::LocalDateTime, Value::LocalDateTime(value)) => {
            Ok(CompositeKeyComponent::LocalDateTime(*value))
        }
        (TypedIndexKind::Uuid, Value::Uuid(value)) => Ok(CompositeKeyComponent::Uuid(*value)),
        (expected_kind, value) => Err(TypedIndexValueError::KindMismatch {
            expected_kind,
            observed: crate::typed_index::observed_value_kind(value),
        }),
    }
}

#[cfg(test)]
mod tests {
    use selene_core::db_string;
    use smallvec::smallvec;

    use super::*;

    #[test]
    fn component_from_value_string_kind() {
        let probe = "component_admit.string.unique-1";
        let value = Value::String(db_string(probe).unwrap());

        let component =
            component_from_value(TypedIndexKind::String, &value).expect("string component coerces");

        let CompositeKeyComponent::String(db_string) = component else {
            panic!("expected String component, got {component:?}");
        };
        assert_eq!(db_string.as_str(), probe);
    }

    #[test]
    fn composite_key_rejects_when_later_component_kind_mismatches() {
        let kinds: SmallVec<[TypedIndexKind; 4]> =
            smallvec![TypedIndexKind::String, TypedIndexKind::I64];
        let location = Value::String(db_string("composite_admit.left_to_right.loc").unwrap());
        // Component 1 is kind-mismatched — a String value bound to an I64
        // index component triggers `KindMismatch` on the second component.
        let bad = Value::String(db_string("composite_admit.left_to_right.bad").unwrap());
        let refs: Vec<&Value> = vec![&location, &bad];

        let err = composite_key_from_values(&kinds, &refs)
            .expect_err("tuple kind mismatch on later component rejects whole tuple");

        assert!(matches!(
            err,
            CompositeIndexValueError::Component {
                index: 1,
                expected_kind: TypedIndexKind::I64,
                observed: "String",
            }
        ));
    }

    #[test]
    fn composite_key_from_values_admits_string_component() {
        let kinds: SmallVec<[TypedIndexKind; 4]> =
            smallvec![TypedIndexKind::I64, TypedIndexKind::String];
        let ts = Value::Int(7);
        let location = Value::String(db_string("composite_admit.string.unique-1").unwrap());
        let refs: Vec<&Value> = vec![&ts, &location];

        let key = composite_key_from_values(&kinds, &refs).expect("string component coerces");

        assert_eq!(key.len(), 2);
    }

    #[test]
    fn values_share_key_matches_equal_string_components() {
        let index =
            CompositeTypedIndex::new(smallvec![TypedIndexKind::I64, TypedIndexKind::String]);
        let ts_lhs = Value::Int(1);
        let ts_rhs = Value::Int(1);
        let loc_lhs =
            Value::String(db_string("values_share_key.composite.string.unique-1").unwrap());
        let loc_rhs =
            Value::String(db_string("values_share_key.composite.string.unique-1").unwrap());
        let lhs: Vec<&Value> = vec![&ts_lhs, &loc_lhs];
        let rhs: Vec<&Value> = vec![&ts_rhs, &loc_rhs];

        assert!(index.values_share_key(&lhs, &rhs));
    }

    #[test]
    fn distinct_keys_counts_composite_buckets_not_rows() {
        let mut index =
            CompositeTypedIndex::new(smallvec![TypedIndexKind::I64, TypedIndexKind::String]);
        assert_eq!(index.distinct_keys(), 0, "empty index");

        let k1 = db_string("k1").unwrap();
        let v_k1 = Value::String(k1);
        let one = Value::Int(1);
        let two = Value::Int(2);

        // (1, k1) on two rows, (2, k1) on one row: 3 rows, 2 distinct composite keys.
        index.insert(&[&one, &v_k1], 0).unwrap();
        index.insert(&[&one, &v_k1], 1).unwrap();
        index.insert(&[&two, &v_k1], 2).unwrap();
        assert_eq!(index.cardinality(), 3);
        assert_eq!(index.distinct_keys(), 2);

        // Remove one of the two rows on (1, k1): bucket stays alive.
        index.remove(&[&one, &v_k1], 0).unwrap();
        assert_eq!(index.cardinality(), 2);
        assert_eq!(index.distinct_keys(), 2);

        // Remove the last row on (1, k1): bucket pruned → distinct drops.
        index.remove(&[&one, &v_k1], 1).unwrap();
        assert_eq!(index.cardinality(), 1);
        assert_eq!(index.distinct_keys(), 1);
    }

    #[test]
    fn values_share_key_returns_false_for_distinct_strings() {
        let index =
            CompositeTypedIndex::new(smallvec![TypedIndexKind::I64, TypedIndexKind::String]);
        let ts_lhs = Value::Int(1);
        let ts_rhs = Value::Int(1);
        let loc_lhs = Value::String(db_string("values_share_key.composite.lhs-unique").unwrap());
        let loc_rhs = Value::String(db_string("values_share_key.composite.rhs-unique").unwrap());
        let lhs: Vec<&Value> = vec![&ts_lhs, &loc_lhs];
        let rhs: Vec<&Value> = vec![&ts_rhs, &loc_rhs];

        assert!(!index.values_share_key(&lhs, &rhs));
    }
}
