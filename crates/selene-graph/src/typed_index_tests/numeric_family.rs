//! [`TypedIndexKind::is_numeric`] against [`Value::is_number`].
//!
//! The two predicates live in different crates and answer the same ISO
//! question (§4.16.5.2, "any two numbers are essentially comparable values")
//! about the two halves of one `(index kind, stored value)` pair. Drift
//! classification asks both and acts on the conjunction, so a disagreement —
//! a numeric kind added on one side only — would be silent.

use super::*;

/// The `Value` variant an index of `kind` is built to key.
///
/// Written as an exhaustive match so a new [`TypedIndexKind`] fails to compile
/// here rather than slipping past the sweep below.
fn backing_value(kind: TypedIndexKind) -> Value {
    match kind {
        TypedIndexKind::Bool => Value::Bool(false),
        TypedIndexKind::I64 => Value::Int(0),
        TypedIndexKind::U64 => Value::Uint(0),
        TypedIndexKind::I128 => Value::Int128(0),
        TypedIndexKind::U128 => Value::Uint128(0),
        TypedIndexKind::Decimal => Value::Decimal(decimal("0")),
        TypedIndexKind::F32 => Value::Float32(0.0),
        TypedIndexKind::F64 => Value::Float(0.0),
        TypedIndexKind::String => {
            Value::String(selene_core::db_string("typed.index.numeric.family").unwrap())
        }
        TypedIndexKind::Date => Value::Date("2024-01-01".parse().unwrap()),
        TypedIndexKind::LocalDateTime => {
            Value::LocalDateTime("2024-01-01T00:00:00".parse().unwrap())
        }
        TypedIndexKind::ZonedDateTime => Value::ZonedDateTime(Box::new(
            "2024-01-01T00:00:00[UTC]".parse().expect("fixture parses"),
        )),
        TypedIndexKind::LocalTime => Value::LocalTime("00:00:00".parse().unwrap()),
        TypedIndexKind::ZonedTime => Value::ZonedTime(Box::new(
            "2024-01-01T00:00:00[UTC]".parse().expect("fixture parses"),
        )),
        TypedIndexKind::Duration => Value::Duration(Box::new("PT1S".parse().unwrap())),
        TypedIndexKind::Uuid => Value::Uuid(uuid::Uuid::nil()),
    }
}

const ALL_KINDS: [TypedIndexKind; 16] = [
    TypedIndexKind::Bool,
    TypedIndexKind::I64,
    TypedIndexKind::U64,
    TypedIndexKind::I128,
    TypedIndexKind::U128,
    TypedIndexKind::Decimal,
    TypedIndexKind::F32,
    TypedIndexKind::F64,
    TypedIndexKind::String,
    TypedIndexKind::Date,
    TypedIndexKind::LocalDateTime,
    TypedIndexKind::ZonedDateTime,
    TypedIndexKind::LocalTime,
    TypedIndexKind::ZonedTime,
    TypedIndexKind::Duration,
    TypedIndexKind::Uuid,
];

#[test]
fn every_kind_keys_its_backing_value() {
    // Also proves `backing_value` names the right variant per kind, which the
    // agreement test below depends on.
    for kind in ALL_KINDS {
        let mut index = TypedIndex::new(kind);
        index
            .insert(&backing_value(kind), 0)
            .unwrap_or_else(|err| panic!("{kind:?} rejected its own backing value: {err:?}"));
        assert_eq!(index.kind(), kind);
    }
}

#[test]
fn index_kind_numeric_agrees_with_value_numeric() {
    for kind in ALL_KINDS {
        assert_eq!(
            kind.is_numeric(),
            backing_value(kind).is_number(),
            "{kind:?} and its backing value disagree about the ISO numeric family",
        );
    }
}

#[test]
fn exactly_the_seven_numeric_kinds_are_numeric() {
    // The shape, not just the count: a swap that made `Bool` numeric and `F32`
    // not would keep the tally at seven.
    let numeric: Vec<TypedIndexKind> = ALL_KINDS
        .into_iter()
        .filter(|kind| kind.is_numeric())
        .collect();
    assert_eq!(
        numeric,
        vec![
            TypedIndexKind::I64,
            TypedIndexKind::U64,
            TypedIndexKind::I128,
            TypedIndexKind::U128,
            TypedIndexKind::Decimal,
            TypedIndexKind::F32,
            TypedIndexKind::F64,
        ],
    );
}
