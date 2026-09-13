use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::*;

#[test]
fn kind_round_trips_for_each_variant() {
    for kind in [
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
    ] {
        assert_eq!(TypedIndex::new(kind).kind(), kind);
    }
}

#[test]
fn kind_rkyv_round_trips_for_each_variant() {
    for kind in [
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
    ] {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&kind).unwrap();
        let round: TypedIndexKind =
            rkyv::from_bytes::<TypedIndexKind, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(round, kind);
    }
}

#[test]
fn not_nan_rejects_nan() {
    assert_eq!(NotNanF32::new(f32::NAN), Err(NotNanError));
    assert_eq!(NotNanF64::new(f64::NAN), Err(NotNanError));
}

#[test]
fn not_nan_collapses_signed_zero_to_one_key() {
    assert_eq!(NotNanF32::new(0.0).unwrap(), NotNanF32::new(-0.0).unwrap());
    assert_eq!(NotNanF64::new(0.0).unwrap(), NotNanF64::new(-0.0).unwrap());

    assert_eq!(
        NotNanF32::new(-0.0).unwrap().get().to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(
        NotNanF64::new(-0.0).unwrap().get().to_bits(),
        0.0_f64.to_bits()
    );
}

#[test]
fn not_nan_signed_zero_hashes_and_orders_as_one_key() {
    fn digest<T: Hash>(value: T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    assert_eq!(
        digest(NotNanF32::new(-0.0).unwrap()),
        digest(NotNanF32::new(0.0).unwrap())
    );
    assert_eq!(
        digest(NotNanF64::new(-0.0).unwrap()),
        digest(NotNanF64::new(0.0).unwrap())
    );

    assert_eq!(
        NotNanF32::new(-0.0)
            .unwrap()
            .cmp(&NotNanF32::new(0.0).unwrap()),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        NotNanF64::new(-0.0)
            .unwrap()
            .cmp(&NotNanF64::new(0.0).unwrap()),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn not_nan_total_order_matches_total_cmp() {
    let values = [f64::NEG_INFINITY, -1.0, 0.0, 1.0, f64::INFINITY]
        .map(|value| NotNanF64::new(value).unwrap());
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));

    let values = [f32::NEG_INFINITY, -1.0, 0.0, 1.0, f32::INFINITY]
        .map(|value| NotNanF32::new(value).unwrap());
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn not_nan_hash_agrees_with_eq() {
    let lhs = NotNanF64::new(42.0).unwrap();
    let rhs = NotNanF64::new(42.0).unwrap();
    let mut lhs_hasher = DefaultHasher::new();
    let mut rhs_hasher = DefaultHasher::new();
    lhs.hash(&mut lhs_hasher);
    rhs.hash(&mut rhs_hasher);
    assert_eq!(lhs, rhs);
    assert_eq!(lhs_hasher.finish(), rhs_hasher.finish());

    let lhs = NotNanF32::new(42.0).unwrap();
    let rhs = NotNanF32::new(42.0).unwrap();
    let mut lhs_hasher = DefaultHasher::new();
    let mut rhs_hasher = DefaultHasher::new();
    lhs.hash(&mut lhs_hasher);
    rhs.hash(&mut rhs_hasher);
    assert_eq!(lhs, rhs);
    assert_eq!(lhs_hasher.finish(), rhs_hasher.finish());
}
