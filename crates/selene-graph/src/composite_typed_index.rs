//! Built-in composite-property value index.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use roaring::RoaringBitmap;
use selene_core::{CoreError, IStr, Value};
use smallvec::SmallVec;

use crate::typed_index::{NotNanError, NotNanF64, TypedIndexKind, TypedIndexValueError};

/// Composite key used by a composite-property index.
pub type CompositeKey = SmallVec<[CompositeKeyComponent; 4]>;

/// One ordered component in a [`CompositeKey`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositeKeyComponent {
    /// Signed integer component.
    I64(i64),
    /// Floating-point component with NaN excluded.
    F64(NotNanF64),
    /// Interned string component.
    String(IStr),
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
    const fn discriminant(self) -> u8 {
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
    ///
    /// `STRING`-component values that arrive as [`Value::ExternalString`]
    /// are admitted into the global [`IStr`] pool here (see
    /// [`composite_key_from_values_admit`]). Cap exhaustion surfaces as
    /// [`CompositeIndexValueError::ComponentAdmissionFailed`].
    pub fn insert(&mut self, values: &[&Value], row: u32) -> Result<(), CompositeIndexValueError> {
        let key = self.key_from_values_admit(values)?;
        self.entries.entry(key).or_default().insert(row);
        Ok(())
    }

    /// Remove `row` from the composite key formed from `values`.
    ///
    /// Uses the admit path so a removal whose only difference is
    /// `String`/`ExternalString` variant still locates the bucket inserted
    /// by the write path.
    pub fn remove(&mut self, values: &[&Value], row: u32) -> Result<(), CompositeIndexValueError> {
        let key = self.key_from_values_admit(values)?;
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

    /// Build a lookup key from the index's ordered component kinds, admitting
    /// new `STRING`-component content into the global [`IStr`] pool if needed.
    ///
    /// Use this from write/maintenance paths. Read paths must call
    /// [`Self::key_from_values_lookup`] instead.
    pub fn key_from_values_admit(
        &self,
        values: &[&Value],
    ) -> Result<CompositeKey, CompositeIndexValueError> {
        composite_key_from_values_admit(&self.kinds, values)
    }

    /// Build a lookup key from the index's ordered component kinds **without
    /// admitting any new content** to the global [`IStr`] pool.
    ///
    /// Returns `Ok(None)` when any `STRING` component is a
    /// [`Value::ExternalString`] whose content has never been admitted —
    /// because no row could be keyed on that content, callers render this
    /// as an empty result rather than admitting just to probe.
    pub fn key_from_values_lookup(
        &self,
        values: &[&Value],
    ) -> Result<Option<CompositeKey>, CompositeIndexValueError> {
        composite_key_from_values_lookup(&self.kinds, values)
    }

    /// Return true when two value tuples address the same key.
    ///
    /// Uses the lookup path so the diff never admits to compare keys; when
    /// either side has a `STRING` component whose content is not yet pooled,
    /// falls through to a pairwise content compare on the raw values.
    pub fn values_share_key(&self, lhs: &[&Value], rhs: &[&Value]) -> bool {
        match (
            self.key_from_values_lookup(lhs),
            self.key_from_values_lookup(rhs),
        ) {
            (Ok(Some(lhs_key)), Ok(Some(rhs_key))) => lhs_key == rhs_key,
            (Err(_), Err(_)) => true,
            (Ok(_), Ok(_)) => vec_raw_value_same(lhs, rhs),
            _ => false,
        }
    }
}

/// Value-admission error for composite index mutation.
///
/// `Clone + Copy + Eq + PartialEq` are intentionally NOT derived: the
/// `ComponentAdmissionFailed` variant carries [`selene_core::CoreError`],
/// which is non-`Copy`/`Clone` by design.
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
    /// A component value's content could not be admitted to the global
    /// [`IStr`] pool (typically because the pool reached its cap).
    ///
    /// Parallel to [`TypedIndexValueError::AdmissionFailed`] for composite
    /// indexes. Promoted by the mutator to a hard
    /// [`crate::error::GraphError::IndexAdmissionExhausted`].
    ComponentAdmissionFailed {
        /// Zero-based component index.
        index: usize,
        /// Registered component kind (always [`TypedIndexKind::String`]
        /// today).
        expected_kind: TypedIndexKind,
        /// Underlying admission failure source.
        reason: CoreError,
    },
}

/// Build a composite key from ordered component kinds and values, admitting
/// new `STRING`-component content into the global [`IStr`] pool when needed.
///
/// Runs two passes (BRIEF-153 fix-cycle C4): pass 1 validates **every**
/// component's kind via the non-admitting [`component_from_value_lookup`]
/// helper, so a kind mismatch on a later component cannot trigger an
/// admission on an earlier `STRING` component. Pass 2 admits and builds
/// the key only when pass 1 cleared. Without this split, the tuple
/// `(ExternalString("x"), Date("bad"))` against `(STRING, I64)` would
/// admit `"x"` to the pool and then error on component 1 — a pool growth
/// DoS amplifier near cap exhaustion.
pub(crate) fn composite_key_from_values_admit(
    kinds: &[TypedIndexKind],
    values: &[&Value],
) -> Result<CompositeKey, CompositeIndexValueError> {
    if kinds.len() != values.len() {
        return Err(CompositeIndexValueError::ArityMismatch {
            expected: kinds.len(),
            observed: values.len(),
        });
    }
    // Pass 1 — admission-free kind check. `Ok(Some(_))` and `Ok(None)`
    // both indicate the value's kind is compatible with the component
    // kind (Ok(None) means a STRING component with unpooled
    // ExternalString content; the kind matches, only admission is
    // deferred).
    for (index, (kind, value)) in kinds.iter().zip(values).enumerate() {
        if let Err(source) = component_from_value_lookup(*kind, value) {
            return Err(CompositeIndexValueError::Component {
                index,
                expected_kind: source.expected_kind(),
                observed: source.observed(),
            });
        }
    }
    // Pass 2 — admit and build the key. Admission errors now surface
    // intact via `ComponentAdmissionFailed`; kind mismatches are
    // impossible because pass 1 cleared them.
    kinds
        .iter()
        .zip(values)
        .enumerate()
        .map(|(index, (kind, value))| {
            component_from_value_admit(*kind, value).map_err(|source| match source {
                TypedIndexValueError::AdmissionFailed { reason, .. } => {
                    CompositeIndexValueError::ComponentAdmissionFailed {
                        index,
                        expected_kind: TypedIndexKind::String,
                        reason,
                    }
                }
                other => CompositeIndexValueError::Component {
                    index,
                    expected_kind: other.expected_kind(),
                    observed: other.observed(),
                },
            })
        })
        .collect()
}

/// Build a composite key from ordered component kinds and values **without
/// admitting new content** to the global [`IStr`] pool.
///
/// Returns `Ok(None)` when any `STRING` component is a
/// [`Value::ExternalString`] whose content has never been admitted.
pub(crate) fn composite_key_from_values_lookup(
    kinds: &[TypedIndexKind],
    values: &[&Value],
) -> Result<Option<CompositeKey>, CompositeIndexValueError> {
    if kinds.len() != values.len() {
        return Err(CompositeIndexValueError::ArityMismatch {
            expected: kinds.len(),
            observed: values.len(),
        });
    }
    let mut key: CompositeKey = SmallVec::with_capacity(kinds.len());
    for (index, (kind, value)) in kinds.iter().zip(values).enumerate() {
        match component_from_value_lookup(*kind, value) {
            Ok(Some(component)) => key.push(component),
            Ok(None) => return Ok(None),
            Err(source) => {
                return Err(CompositeIndexValueError::Component {
                    index,
                    expected_kind: source.expected_kind(),
                    observed: source.observed(),
                });
            }
        }
    }
    Ok(Some(key))
}

fn component_from_value_admit(
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
        (TypedIndexKind::String, Value::String(value)) => Ok(CompositeKeyComponent::String(*value)),
        (TypedIndexKind::String, Value::ExternalString(value)) => {
            match selene_core::intern(value.as_ref()) {
                Ok(interned) => Ok(CompositeKeyComponent::String(interned)),
                Err(reason) => Err(TypedIndexValueError::AdmissionFailed {
                    expected_kind: TypedIndexKind::String,
                    reason,
                }),
            }
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

fn component_from_value_lookup(
    kind: TypedIndexKind,
    value: &Value,
) -> Result<Option<CompositeKeyComponent>, TypedIndexValueError> {
    match (kind, value) {
        (TypedIndexKind::I64, Value::Int(value)) => Ok(Some(CompositeKeyComponent::I64(*value))),
        (TypedIndexKind::F64, Value::Float(value)) => NotNanF64::new(*value)
            .map(|key| Some(CompositeKeyComponent::F64(key)))
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        (TypedIndexKind::String, Value::String(value)) => {
            Ok(Some(CompositeKeyComponent::String(*value)))
        }
        (TypedIndexKind::String, Value::ExternalString(value)) => {
            Ok(selene_core::lookup(value.as_ref()).map(CompositeKeyComponent::String))
        }
        (TypedIndexKind::Date, Value::Date(value)) => Ok(Some(CompositeKeyComponent::Date(*value))),
        (TypedIndexKind::LocalDateTime, Value::LocalDateTime(value)) => {
            Ok(Some(CompositeKeyComponent::LocalDateTime(*value)))
        }
        (TypedIndexKind::Uuid, Value::Uuid(value)) => Ok(Some(CompositeKeyComponent::Uuid(*value))),
        (expected_kind, value) => Err(TypedIndexValueError::KindMismatch {
            expected_kind,
            observed: crate::typed_index::observed_value_kind(value),
        }),
    }
}

/// Pairwise raw-value comparison used by [`CompositeTypedIndex::values_share_key`]
/// when at least one side has an unpoolable `STRING` component. Matches the
/// content-equality semantics of the single-key
/// [`crate::typed_index`]`::raw_value_same` helper.
fn vec_raw_value_same(lhs: &[&Value], rhs: &[&Value]) -> bool {
    if lhs.len() != rhs.len() {
        return false;
    }
    lhs.iter().zip(rhs).all(|(l, r)| raw_value_same(l, r))
}

fn raw_value_same(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Float(lhs), Value::Float(rhs)) => lhs.to_bits() == rhs.to_bits(),
        (Value::Float32(lhs), Value::Float32(rhs)) => lhs.to_bits() == rhs.to_bits(),
        (Value::String(lhs), Value::String(rhs)) => lhs == rhs,
        (Value::ExternalString(lhs), Value::ExternalString(rhs)) => lhs.as_ref() == rhs.as_ref(),
        (Value::String(lhs), Value::ExternalString(rhs)) => lhs.as_str() == rhs.as_ref(),
        (Value::ExternalString(lhs), Value::String(rhs)) => lhs.as_ref() == rhs.as_str(),
        _ => lhs == rhs,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::{intern, lookup};
    use smallvec::smallvec;

    use super::*;

    #[test]
    fn component_from_value_admit_external_string_string_kind() {
        let probe = "component_admit.external.unique-1";
        assert!(lookup(probe).is_none());
        let value = Value::ExternalString(Arc::<str>::from(probe));

        let component = component_from_value_admit(TypedIndexKind::String, &value)
            .expect("admission succeeds under cap");

        let CompositeKeyComponent::String(istr) = component else {
            panic!("expected String component, got {component:?}");
        };
        assert_eq!(istr.as_str(), probe);
        assert!(lookup(probe).is_some());
    }

    #[test]
    fn component_from_value_lookup_external_string_not_in_pool_returns_none() {
        let probe = "component_lookup.external.unique-not-admitted";
        assert!(lookup(probe).is_none());
        let value = Value::ExternalString(Arc::<str>::from(probe));

        let result =
            component_from_value_lookup(TypedIndexKind::String, &value).expect("kind matches");

        assert!(result.is_none());
        assert!(lookup(probe).is_none(), "lookup must not admit");
    }

    #[test]
    fn composite_key_from_values_lookup_returns_none_when_any_component_missing() {
        let kinds: SmallVec<[TypedIndexKind; 4]> =
            smallvec![TypedIndexKind::I64, TypedIndexKind::String];
        // Component 1 (STRING) is unpoolable; whole key resolves to None.
        let probe = "composite_lookup.external.unique-not-admitted";
        assert!(lookup(probe).is_none());
        let ts = Value::Int(5);
        let location = Value::ExternalString(Arc::<str>::from(probe));
        let refs: Vec<&Value> = vec![&ts, &location];

        let result = composite_key_from_values_lookup(&kinds, &refs).expect("arity + kinds match");

        assert!(result.is_none());
        assert!(lookup(probe).is_none());
    }

    #[test]
    fn composite_key_admit_does_not_admit_when_later_component_kind_mismatches() {
        // BRIEF-153 fix-cycle C4: two-pass admission. The tuple's later
        // component is kind-mismatched, so the earlier STRING component's
        // ExternalString content MUST NOT be admitted to the global pool.
        let kinds: SmallVec<[TypedIndexKind; 4]> =
            smallvec![TypedIndexKind::String, TypedIndexKind::I64];
        let probe = "composite_admit.left_to_right.unique-never-pooled";
        assert!(lookup(probe).is_none());
        let location = Value::ExternalString(Arc::<str>::from(probe));
        // Component 1 is kind-mismatched — a String value bound to an I64
        // index component triggers `KindMismatch` on the second component.
        let bad = Value::String(intern("composite_admit.left_to_right.bad").unwrap());
        let refs: Vec<&Value> = vec![&location, &bad];

        let err = composite_key_from_values_admit(&kinds, &refs)
            .expect_err("tuple kind mismatch on later component rejects whole tuple");

        assert!(matches!(
            err,
            CompositeIndexValueError::Component {
                index: 1,
                expected_kind: TypedIndexKind::I64,
                observed: "String",
            }
        ));
        assert!(
            lookup(probe).is_none(),
            "pass-1 kind check must run before any STRING admission"
        );
    }

    #[test]
    fn composite_key_from_values_admit_admits_string_component() {
        let kinds: SmallVec<[TypedIndexKind; 4]> =
            smallvec![TypedIndexKind::I64, TypedIndexKind::String];
        let probe = "composite_admit.external.unique-1";
        assert!(lookup(probe).is_none());
        let ts = Value::Int(7);
        let location = Value::ExternalString(Arc::<str>::from(probe));
        let refs: Vec<&Value> = vec![&ts, &location];

        let key =
            composite_key_from_values_admit(&kinds, &refs).expect("admission succeeds under cap");

        assert_eq!(key.len(), 2);
        assert!(lookup(probe).is_some());
    }

    #[test]
    fn values_share_key_uses_raw_compare_when_string_component_unpoolable() {
        let index =
            CompositeTypedIndex::new(smallvec![TypedIndexKind::I64, TypedIndexKind::String]);
        let probe = "values_share_key.composite.external.unique-1";
        assert!(lookup(probe).is_none());
        let ts_lhs = Value::Int(1);
        let ts_rhs = Value::Int(1);
        let loc_lhs = Value::ExternalString(Arc::<str>::from(probe));
        let loc_rhs = Value::ExternalString(Arc::<str>::from(probe));
        let lhs: Vec<&Value> = vec![&ts_lhs, &loc_lhs];
        let rhs: Vec<&Value> = vec![&ts_rhs, &loc_rhs];

        assert!(index.values_share_key(&lhs, &rhs));
        assert!(lookup(probe).is_none(), "diff must not admit");
    }

    #[test]
    fn distinct_keys_counts_composite_buckets_not_rows() {
        let mut index =
            CompositeTypedIndex::new(smallvec![TypedIndexKind::I64, TypedIndexKind::String]);
        assert_eq!(index.distinct_keys(), 0, "empty index");

        let k1 = intern("k1").unwrap();
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
    fn values_share_key_returns_false_for_distinct_unpoolable_strings() {
        let index =
            CompositeTypedIndex::new(smallvec![TypedIndexKind::I64, TypedIndexKind::String]);
        let lhs_content = "values_share_key.composite.external.lhs-unique";
        let rhs_content = "values_share_key.composite.external.rhs-unique";
        assert!(lookup(lhs_content).is_none());
        assert!(lookup(rhs_content).is_none());
        let ts_lhs = Value::Int(1);
        let ts_rhs = Value::Int(1);
        let loc_lhs = Value::ExternalString(Arc::<str>::from(lhs_content));
        let loc_rhs = Value::ExternalString(Arc::<str>::from(rhs_content));
        let lhs: Vec<&Value> = vec![&ts_lhs, &loc_lhs];
        let rhs: Vec<&Value> = vec![&ts_rhs, &loc_rhs];

        assert!(!index.values_share_key(&lhs, &rhs));
        assert!(lookup(lhs_content).is_none());
        assert!(lookup(rhs_content).is_none());
    }
}
