//! Canonical key wrappers for runtime DISTINCT, aggregate DISTINCT, and joins.
//!
//! `DistinctRowKey` is variant-strict and follows `Value::PartialEq`.
//! `RuntimeEqKey` follows the runtime row-key comparator used by pattern joins:
//! cross-type `Int`/`Uint`/`Float`/`Float32` values compare by lossless numeric
//! equality, and interned/external strings compare by string contents. Both hash
//! paths normalize signed zero and NaN payloads so any values that compare equal
//! under their key regime hash identically.

use std::{
    hash::{Hash, Hasher},
    mem,
};

use selene_core::{PathSegment, Record, Value};

use super::value_compare;

/// Variant-strict key for top-level `DISTINCT` and structural row dedup.
#[derive(Clone, Debug)]
pub(crate) struct DistinctRowKey(pub(crate) Vec<Value>);

impl PartialEq for DistinctRowKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for DistinctRowKey {}

impl Hash for DistinctRowKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for value in &self.0 {
            hash_value_variant_strict(value, state);
        }
    }
}

/// Runtime-equality key for aggregate `DISTINCT` and hash-join key probes.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeEqKey(pub(crate) Vec<Value>);

impl RuntimeEqKey {
    pub(crate) fn from_row(row: Vec<Value>) -> Self {
        Self(row)
    }
}

impl PartialEq for RuntimeEqKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(&other.0)
                .all(|(lhs, rhs)| runtime_values_equal(lhs, rhs))
    }
}

impl Eq for RuntimeEqKey {}

impl Hash for RuntimeEqKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for value in &self.0 {
            hash_value_runtime_eq(value, state);
        }
    }
}

fn runtime_values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        _ => value_compare::equal_non_null(lhs, rhs),
    }
}

fn hash_value_runtime_eq<H: Hasher>(value: &Value, state: &mut H) {
    if hash_runtime_numeric(value, state) {
        return;
    }
    match value {
        Value::String(value) => {
            "runtime-string".hash(state);
            value.as_str().hash(state);
        }
        Value::ExternalString(value) => {
            "runtime-string".hash(state);
            value.as_ref().hash(state);
        }
        _ => hash_value_variant_strict(value, state),
    }
}

fn hash_value_variant_strict<H: Hasher>(value: &Value, state: &mut H) {
    mem::discriminant(value).hash(state);
    match value {
        Value::Bool(value) => value.hash(state),
        Value::Int(value) => value.hash(state),
        Value::Uint(value) => value.hash(state),
        Value::Int128(value) => value.hash(state),
        Value::Uint128(value) => value.hash(state),
        Value::Float(value) => hash_f64_canonical(*value, state),
        Value::Float32(value) => hash_f32_canonical(*value, state),
        Value::Decimal(value) => value.hash(state),
        Value::String(value) => value.hash(state),
        Value::ExternalString(value) => value.hash(state),
        Value::Bytes(value) => value.as_ref().hash(state),
        Value::List(values) => {
            values.len().hash(state);
            for value in values {
                hash_value_variant_strict(value, state);
            }
        }
        Value::Record(record) => hash_record(record, state),
        Value::RecordTyped(record) => {
            record.type_id.hash(state);
            record.values.len().hash(state);
            for value in &record.values {
                value.is_some().hash(state);
                if let Some(value) = value {
                    hash_value_variant_strict(value, state);
                }
            }
        }
        Value::Path(path) => {
            path.graph.hash(state);
            path.start.hash(state);
            path.segments.len().hash(state);
            for segment in &path.segments {
                hash_path_segment(segment, state);
            }
        }
        Value::NodeRef(value) => value.hash(state),
        Value::EdgeRef(value) => value.hash(state),
        Value::GraphRef(value) => value.hash(state),
        Value::TableRef(value) => value.hash(state),
        Value::ZonedDateTime(value) => value.hash(state),
        Value::LocalDateTime(value) => value.hash(state),
        Value::Date(value) => value.hash(state),
        Value::ZonedTime(value) => value.hash(state),
        Value::LocalTime(value) => value.hash(state),
        Value::Duration(value) => (
            value.get_years(),
            value.get_months(),
            value.get_weeks(),
            value.get_days(),
            value.get_hours(),
            value.get_minutes(),
            value.get_seconds(),
            value.get_milliseconds(),
            value.get_microseconds(),
            value.get_nanoseconds(),
        )
            .hash(state),
        Value::Extended { type_id, payload } => {
            type_id.hash(state);
            payload.as_ref().hash(state);
        }
        Value::Null => {}
        Value::Uuid(value) => value.hash(state),
        _ => format!("{value:?}").hash(state),
    }
}

fn hash_record<H: Hasher>(record: &Record, state: &mut H) {
    mem::discriminant(record).hash(state);
    match record {
        Record::Open(fields) => {
            fields.len().hash(state);
            for (name, value) in fields {
                name.hash(state);
                hash_value_variant_strict(value, state);
            }
        }
        _ => format!("{record:?}").hash(state),
    }
}

fn hash_path_segment<H: Hasher>(segment: &PathSegment, state: &mut H) {
    segment.edge.hash(state);
    segment.direction.hash(state);
    segment.node.hash(state);
}

fn hash_f64_canonical<H: Hasher>(value: f64, state: &mut H) {
    if value == 0.0 {
        0_u64.hash(state);
    } else if value.is_nan() {
        u64::MAX.hash(state);
    } else {
        value.to_bits().hash(state);
    }
}

fn hash_f32_canonical<H: Hasher>(value: f32, state: &mut H) {
    if value == 0.0 {
        0_u32.hash(state);
    } else if value.is_nan() {
        u32::MAX.hash(state);
    } else {
        value.to_bits().hash(state);
    }
}

fn hash_runtime_numeric<H: Hasher>(value: &Value, state: &mut H) -> bool {
    match value {
        Value::Int(value) => {
            "runtime-number".hash(state);
            hash_binary_number(
                value.is_negative(),
                u128::from(value.unsigned_abs()),
                0,
                state,
            );
            true
        }
        Value::Uint(value) => {
            "runtime-number".hash(state);
            hash_binary_number(false, u128::from(*value), 0, state);
            true
        }
        Value::Float(value) => {
            "runtime-number".hash(state);
            hash_f64_runtime_numeric(*value, state);
            true
        }
        Value::Float32(value) => {
            "runtime-number".hash(state);
            hash_f32_runtime_numeric(*value, state);
            true
        }
        _ => false,
    }
}

fn hash_f64_runtime_numeric<H: Hasher>(value: f64, state: &mut H) {
    if value == 0.0 {
        hash_binary_number(false, 0, 0, state);
        return;
    }
    let bits = value.to_bits();
    let negative = (bits >> 63) != 0;
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    if exponent == 0x7ff {
        if fraction == 0 {
            "infinity".hash(state);
            negative.hash(state);
        } else {
            "nan".hash(state);
        }
    } else if exponent == 0 {
        hash_binary_number(negative, u128::from(fraction), 1 - 1023 - 52, state);
    } else {
        hash_binary_number(
            negative,
            u128::from((1_u64 << 52) | fraction),
            exponent - 1023 - 52,
            state,
        );
    }
}

fn hash_f32_runtime_numeric<H: Hasher>(value: f32, state: &mut H) {
    if value == 0.0 {
        hash_binary_number(false, 0, 0, state);
        return;
    }
    let bits = value.to_bits();
    let negative = (bits >> 31) != 0;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let fraction = bits & ((1_u32 << 23) - 1);
    if exponent == 0xff {
        if fraction == 0 {
            "infinity".hash(state);
            negative.hash(state);
        } else {
            "nan".hash(state);
        }
    } else if exponent == 0 {
        hash_binary_number(negative, u128::from(fraction), 1 - 127 - 23, state);
    } else {
        hash_binary_number(
            negative,
            u128::from((1_u32 << 23) | fraction),
            exponent - 127 - 23,
            state,
        );
    }
}

fn hash_binary_number<H: Hasher>(
    negative: bool,
    mut significand: u128,
    mut exponent: i32,
    state: &mut H,
) {
    if significand == 0 {
        "zero".hash(state);
        return;
    }
    let shift = significand.trailing_zeros();
    significand >>= shift;
    exponent += shift as i32;
    "finite".hash(state);
    negative.hash(state);
    significand.hash(state);
    exponent.hash(state);
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, hash_map::DefaultHasher},
        hash::{Hash, Hasher},
        sync::Arc,
    };

    use proptest::{prelude::*, test_runner::Config};
    use selene_core::{Record, Value, intern_with_admission};
    use smallvec::smallvec;

    use super::{DistinctRowKey, RuntimeEqKey, runtime_values_equal};

    fn key_hash(value: &impl Hash) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn value_key_hash_eq_invariant_signed_zero() {
        let lhs = DistinctRowKey(vec![Value::Float(0.0)]);
        let rhs = DistinctRowKey(vec![Value::Float(-0.0)]);

        assert_eq!(lhs, rhs);
        assert_eq!(key_hash(&lhs), key_hash(&rhs));
    }

    #[test]
    fn value_key_hash_eq_invariant_nan() {
        let lhs = DistinctRowKey(vec![Value::Float(f64::from_bits(0x7ff8_0000_0000_0001))]);
        let rhs = DistinctRowKey(vec![Value::Float(f64::from_bits(0x7ff8_0000_0000_0002))]);

        assert_eq!(lhs, rhs);
        assert_eq!(key_hash(&lhs), key_hash(&rhs));
    }

    #[test]
    fn distinct_row_key_keeps_int_and_float_apart() {
        let int = DistinctRowKey(vec![Value::Int(1)]);
        let float = DistinctRowKey(vec![Value::Float(1.0)]);

        assert_ne!(int, float);
    }

    #[test]
    fn runtime_eq_key_collapses_cross_type_numerics() {
        let int = RuntimeEqKey::from_row(vec![Value::Int(1)]);
        let uint = RuntimeEqKey::from_row(vec![Value::Uint(1)]);
        let float = RuntimeEqKey::from_row(vec![Value::Float(1.0)]);
        let float32 = RuntimeEqKey::from_row(vec![Value::Float32(1.0)]);

        assert_eq!(int, uint);
        assert_eq!(int, float);
        assert_eq!(int, float32);
        assert_eq!(key_hash(&int), key_hash(&uint));
        assert_eq!(key_hash(&int), key_hash(&float));
        assert_eq!(key_hash(&int), key_hash(&float32));
    }

    #[test]
    fn runtime_eq_key_hashes_interned_and_external_strings_by_content() {
        let interned = RuntimeEqKey::from_row(vec![Value::String(
            intern_with_admission("same")
                .expect("test string interns")
                .0,
        )]);
        let external = RuntimeEqKey::from_row(vec![Value::ExternalString(Arc::from("same"))]);

        assert_eq!(interned, external);
        assert_eq!(key_hash(&interned), key_hash(&external));
    }

    #[test]
    fn runtime_eq_key_dedups_record_with_null_by_rust_equality() {
        let key = intern_with_admission("x").expect("test key interns").0;
        let record = Value::Record(Box::new(Record::Open(smallvec![(key, Value::Null)])));
        let mut map = HashMap::new();

        assert_eq!(record, record.clone());
        map.insert(RuntimeEqKey::from_row(vec![record.clone()]), 1);
        map.insert(RuntimeEqKey::from_row(vec![record]), 2);

        assert_eq!(map.len(), 1);
        assert_eq!(map.values().copied().collect::<Vec<_>>(), vec![2]);
    }

    proptest! {
        #![proptest_config(Config::with_cases(256))]

        #[test]
        fn runtime_eq_key_parity_with_runtime_equal(
            lhs in runtime_value_strategy(),
            rhs in runtime_value_strategy(),
        ) {
            let expected = runtime_values_equal(&lhs, &rhs);
            let lhs_key = RuntimeEqKey::from_row(vec![lhs]);
            let rhs_key = RuntimeEqKey::from_row(vec![rhs]);

            prop_assert_eq!(lhs_key == rhs_key, expected);
        }
    }

    fn runtime_value_strategy() -> BoxedStrategy<Value> {
        prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            (-1000_i64..1000).prop_map(Value::Int),
            (0_u64..1000).prop_map(Value::Uint),
            (-1000_i64..1000).prop_map(|value| Value::Float(value as f64)),
            (-1000_i32..1000).prop_map(|value| Value::Float(value as f64 + 0.5)),
            Just(Value::Float(0.0)),
            Just(Value::Float(-0.0)),
            Just(Value::Float(f64::NAN)),
            Just(Value::Float(f64::INFINITY)),
            (-1000_i16..1000).prop_map(|value| Value::Float32(value as f32)),
            Just(Value::Float32(-0.0)),
            Just(Value::Float32(f32::NAN)),
            prop::sample::select(vec!["a", "b", "same"]).prop_map(|value| {
                Value::String(intern_with_admission(value).expect("test string interns").0)
            }),
            prop::sample::select(vec!["a", "b", "same"])
                .prop_map(|value| { Value::ExternalString(Arc::from(value)) }),
        ]
        .boxed()
    }
}
