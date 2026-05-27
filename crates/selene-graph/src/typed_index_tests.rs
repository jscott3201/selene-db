use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use jiff::civil::{date, datetime};
use roaring::RoaringBitmap;
use selene_core::{Value, intern, lookup};

use super::*;

fn row_index(index: &TypedIndex, value: &Value) -> RoaringBitmap {
    index.lookup_eq(value).expect("kind matches").into_owned()
}

#[test]
fn kind_round_trips_for_each_variant() {
    for kind in [
        TypedIndexKind::I64,
        TypedIndexKind::F64,
        TypedIndexKind::String,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
        TypedIndexKind::Uuid,
    ] {
        assert_eq!(TypedIndex::new(kind).kind(), kind);
    }
}

#[test]
fn kind_rkyv_round_trips_for_each_variant() {
    for kind in [
        TypedIndexKind::I64,
        TypedIndexKind::F64,
        TypedIndexKind::String,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
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
    assert_eq!(NotNanF64::new(f64::NAN), Err(NotNanError));
}

#[test]
fn not_nan_preserves_zero_sign_as_distinct_keys() {
    assert_ne!(NotNanF64::new(0.0).unwrap(), NotNanF64::new(-0.0).unwrap());
}

#[test]
fn not_nan_total_order_matches_total_cmp() {
    let values = [f64::NEG_INFINITY, -1.0, -0.0, 0.0, 1.0, f64::INFINITY]
        .map(|value| NotNanF64::new(value).unwrap());
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
}

#[test]
fn insert_remove_round_trips_for_each_kind() {
    let string = intern("typed-index.string").unwrap();
    let uuid = uuid::Uuid::from_u128(7);
    let cases = [
        (TypedIndexKind::I64, Value::Int(7)),
        (TypedIndexKind::F64, Value::Float(7.0)),
        (TypedIndexKind::String, Value::String(string)),
        (TypedIndexKind::Date, Value::Date(date(2026, 5, 7))),
        (
            TypedIndexKind::LocalDateTime,
            Value::LocalDateTime(datetime(2026, 5, 7, 12, 30, 0, 0)),
        ),
        (TypedIndexKind::Uuid, Value::Uuid(uuid)),
    ];
    for (kind, value) in cases {
        let mut index = TypedIndex::new(kind);
        index.insert(&value, 3).unwrap();
        assert!(row_index(&index, &value).contains(3));
        index.remove(&value, 3).unwrap();
        assert!(row_index(&index, &value).is_empty());
        assert_eq!(index.cardinality(), 0);
    }
}

#[test]
fn lookup_eq_returns_cow_variants_for_hit_and_empty_match() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    index.insert(&Value::Int(7), 3).unwrap();

    let hit = index.lookup_eq(&Value::Int(7)).expect("kind matches");
    assert!(matches!(hit, std::borrow::Cow::Borrowed(_)));
    assert!(hit.contains(3));

    let missing = index.lookup_eq(&Value::Int(8)).expect("kind matches");
    assert!(matches!(missing, std::borrow::Cow::Owned(_)));
    assert!(missing.is_empty());

    assert!(
        index
            .lookup_eq(&Value::String(intern("wrong").unwrap()))
            .is_none()
    );
}

#[test]
fn insert_errors_on_kind_mismatch() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    let err = index.insert(&Value::String(intern("wrong-kind").unwrap()), 0);
    assert!(matches!(
        err,
        Err(TypedIndexValueError::KindMismatch {
            expected_kind: TypedIndexKind::I64,
            observed: "String"
        })
    ));
}

#[test]
fn float_insert_errors_on_nan() {
    let mut index = TypedIndex::new(TypedIndexKind::F64);
    let err = index.insert(&Value::Float(f64::NAN), 0);
    assert!(matches!(
        err,
        Err(TypedIndexValueError::NaN {
            expected_kind: TypedIndexKind::F64
        })
    ));
}

#[test]
fn cardinality_sums_across_keys_and_prunes_empty_keys() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    index.insert(&Value::Int(1), 0).unwrap();
    index.insert(&Value::Int(1), 1).unwrap();
    index.insert(&Value::Int(2), 2).unwrap();
    assert_eq!(index.cardinality(), 3);
    index.remove(&Value::Int(1), 0).unwrap();
    index.remove(&Value::Int(1), 1).unwrap();
    assert_eq!(index.cardinality(), 1);
    assert!(matches!(&index, TypedIndex::I64(inner) if !inner.contains_key(&1)));
}

#[test]
fn range_scan_honors_included_and_excluded_bounds() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    for (row, value) in [(0, 1), (1, 2), (2, 3), (3, 4)] {
        index.insert(&Value::Int(value), row).unwrap();
    }
    let result = index
        .lookup_range(Value::Int(2)..Value::Int(4))
        .expect("range kind matches");
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(!result.contains(0));
    assert!(!result.contains(3));
}

#[test]
fn prefix_scan_matches_string_keys_only() {
    let alpha = intern("typed-index.prefix.alpha").unwrap();
    let beta = intern("typed-index.beta").unwrap();
    let mut index = TypedIndex::new(TypedIndexKind::String);
    index.insert(&Value::String(alpha), 0).unwrap();
    index.insert(&Value::String(beta), 1).unwrap();
    let result = index.lookup_prefix("typed-index.prefix").unwrap();
    assert!(result.contains(0));
    assert!(!result.contains(1));
    assert!(
        TypedIndex::new(TypedIndexKind::I64)
            .lookup_prefix("typed")
            .is_none()
    );
}

#[test]
fn typed_key_admit_external_string_returns_string_key() {
    // ExternalString admits into the global IStr pool on the write path so
    // a STRING-kind index can key on it (BRIEF-153 carve-out).
    let probe = Arc::<str>::from("typed_key_admit.external.unique-1");
    assert!(lookup(probe.as_ref()).is_none());
    let value = Value::ExternalString(Arc::clone(&probe));

    let key = typed_key_admit(&value, TypedIndexKind::String)
        .expect("admission succeeds under the pool cap");

    let TypedKey::String(istr) = key else {
        panic!("expected TypedKey::String, got {key:?}");
    };
    assert_eq!(istr.as_str(), probe.as_ref());
    assert!(lookup(probe.as_ref()).is_some());
}

#[test]
fn typed_key_admit_non_string_kind_with_external_string_does_not_admit() {
    // BRIEF-153 fix-cycle C3: probing a non-STRING-kind admit path with a
    // `Value::ExternalString` must fail KindMismatch BEFORE admitting the
    // content to the global IStr pool — otherwise probing an I64 index with
    // arbitrary ExternalString content amplifies into pool growth.
    let probe = "typed_key_admit.kind_mismatch.unique-never-pooled";
    assert!(lookup(probe).is_none());
    let value = Value::ExternalString(Arc::<str>::from(probe));

    for kind in [
        TypedIndexKind::I64,
        TypedIndexKind::F64,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
        TypedIndexKind::Uuid,
    ] {
        let err = typed_key_admit(&value, kind).expect_err("non-STRING kind rejects");
        assert!(matches!(
            err,
            TypedIndexValueError::KindMismatch {
                observed: "ExternalString",
                ..
            }
        ));
    }
    assert!(
        lookup(probe).is_none(),
        "no admission must have happened on any non-STRING kind probe"
    );
}

#[test]
fn typed_key_lookup_external_string_returns_none_when_not_in_pool() {
    // Lookup MUST NOT admit (read-path no-admit; BRIEF-153 bar 2).
    let probe = Arc::<str>::from("typed_key_lookup.external.unique-not-admitted");
    assert!(lookup(probe.as_ref()).is_none());
    let value = Value::ExternalString(Arc::clone(&probe));

    let result = typed_key_lookup(&value).expect("kind matches");

    assert!(result.is_none(), "probe content was not in the pool");
    assert!(
        lookup(probe.as_ref()).is_none(),
        "lookup must not admit a new IStr"
    );
}

#[test]
fn typed_key_lookup_external_string_returns_some_when_pre_admitted() {
    // After admitting the content separately, lookup resolves to the pool
    // handle without re-admitting.
    let content = "typed_key_lookup.external.unique-pre-admitted";
    let admitted = intern(content).unwrap();
    let value = Value::ExternalString(Arc::<str>::from(content));

    let result = typed_key_lookup(&value).expect("kind matches");

    let Some(TypedKey::String(istr)) = result else {
        panic!("expected Ok(Some(String(_))), got {result:?}");
    };
    assert_eq!(istr, admitted);
}

#[test]
fn lookup_eq_returns_none_for_kind_drift_under_open_graph() {
    // BRIEF-153 fix-cycle C1: an open-graph index can have rows stored
    // with values that don't match its declared kind (the commit path
    // logs and skips kind drift). A query bound to `Value::ExternalString`
    // against a non-STRING index must return `None` from `lookup_eq` so
    // the caller drops to a linear scan + cross-variant `value_compare`.
    // Returning `Some(empty)` would silently lose the drifted row.
    let index = TypedIndex::new(TypedIndexKind::I64);
    let probe_content = "lookup_eq.kind_drift.never_pooled";
    assert!(lookup(probe_content).is_none());

    let result = index.lookup_eq(&Value::ExternalString(Arc::<str>::from(probe_content)));

    assert!(
        result.is_none(),
        "non-STRING-kind index with ExternalString probe must return None (scan fallback), \
         got Some(_)"
    );
    assert!(
        lookup(probe_content).is_none(),
        "lookup_eq must not admit even on the kind-mismatch path"
    );
}

#[test]
fn lookup_eq_with_external_string_does_not_admit_unknown_content() {
    // Read-path symmetry test (BRIEF-153 bar 2): populate via Value::String,
    // probe via a fresh Value::ExternalString whose content was never
    // admitted; assert empty result AND the probe content is still absent
    // from the global pool afterward.
    let mut index = TypedIndex::new(TypedIndexKind::String);
    let admitted = intern("lookup_eq.external.admitted").unwrap();
    index.insert(&Value::String(admitted), 7).unwrap();

    let probe_content = "lookup_eq.external.unique-not-admitted";
    assert!(lookup(probe_content).is_none());
    let probe = Value::ExternalString(Arc::<str>::from(probe_content));

    let result = index.lookup_eq(&probe).expect("kind matches");

    assert!(
        result.is_empty(),
        "no row could be keyed on unadmitted content"
    );
    assert!(
        lookup(probe_content).is_none(),
        "lookup_eq must not admit the probe content"
    );
}

#[test]
fn lookup_eq_external_string_finds_admitted_row_via_pool_handle() {
    // Variant-cross test: a row inserted as Value::String can still be
    // located by a probe bound as Value::ExternalString with matching
    // content, because typed_key_lookup resolves the content through the
    // existing IStr pool.
    let mut index = TypedIndex::new(TypedIndexKind::String);
    let content = "lookup_eq.external.cross-variant";
    let istr = intern(content).unwrap();
    index.insert(&Value::String(istr), 3).unwrap();

    let probe = Value::ExternalString(Arc::<str>::from(content));
    let result = index.lookup_eq(&probe).expect("kind matches");

    assert!(result.contains(3));
}

#[test]
fn values_share_key_falls_through_for_unpoolable_external_string() {
    // Diff path MUST NOT admit (BRIEF-153 §B.1). Two distinct fresh
    // ExternalString contents — neither in the pool — must NOT share a
    // key, so the update fires a write that admits through the write path.
    let index = TypedIndex::new(TypedIndexKind::String);
    let lhs_content = "values_share_key.external.lhs-unique";
    let rhs_content = "values_share_key.external.rhs-unique";
    assert!(lookup(lhs_content).is_none());
    assert!(lookup(rhs_content).is_none());
    let lhs = Value::ExternalString(Arc::<str>::from(lhs_content));
    let rhs = Value::ExternalString(Arc::<str>::from(rhs_content));

    assert!(!index.values_share_key(&lhs, &rhs));
    assert!(
        lookup(lhs_content).is_none() && lookup(rhs_content).is_none(),
        "values_share_key must not admit either operand"
    );
}

#[test]
fn values_share_key_returns_true_for_same_external_string_content() {
    // Identical raw content from two ExternalString Arcs — neither in the
    // pool — falls through to the raw-value comparison and matches.
    let index = TypedIndex::new(TypedIndexKind::String);
    let content = "values_share_key.external.same-content-unique";
    assert!(lookup(content).is_none());
    let lhs = Value::ExternalString(Arc::<str>::from(content));
    let rhs = Value::ExternalString(Arc::<str>::from(content));

    assert!(index.values_share_key(&lhs, &rhs));
    assert!(
        lookup(content).is_none(),
        "values_share_key must not admit even on equality match"
    );
}

#[test]
fn string_range_returns_none_for_runtime_fallback() {
    let alpha = intern("typed-index.range.alpha").unwrap();
    let charlie = intern("typed-index.range.charlie").unwrap();
    let mut index = TypedIndex::new(TypedIndexKind::String);
    index.insert(&Value::String(alpha), 0).unwrap();
    index.insert(&Value::String(charlie), 1).unwrap();

    assert!(
        index
            .lookup_range(Value::String(alpha)..Value::String(charlie))
            .is_none(),
        "String ranges cannot use IStr-handle BTreeMap order"
    );
}
