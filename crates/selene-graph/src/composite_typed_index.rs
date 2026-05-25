//! Built-in composite-property value index.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use roaring::RoaringBitmap;
use selene_core::{IStr, Value};
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
    #[must_use]
    pub fn cardinality(&self) -> u64 {
        self.entries.values().map(RoaringBitmap::len).sum()
    }

    /// Iterate composite-key buckets and their matching row bitmaps.
    pub fn entries(&self) -> impl Iterator<Item = (&CompositeKey, &RoaringBitmap)> {
        self.entries.iter()
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

    /// Build a lookup key from the index's ordered component kinds.
    pub fn key_from_values(
        &self,
        values: &[&Value],
    ) -> Result<CompositeKey, CompositeIndexValueError> {
        composite_key_from_values(&self.kinds, values)
    }

    /// Return true when two value tuples address the same key.
    pub fn values_share_key(&self, lhs: &[&Value], rhs: &[&Value]) -> bool {
        match (self.key_from_values(lhs), self.key_from_values(rhs)) {
            (Ok(lhs), Ok(rhs)) => lhs == rhs,
            (Err(_), Err(_)) => true,
            _ => false,
        }
    }
}

/// Value-admission error for composite index mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        (TypedIndexKind::String, Value::String(value)) => Ok(CompositeKeyComponent::String(*value)),
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
