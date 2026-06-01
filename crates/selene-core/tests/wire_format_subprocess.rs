//! Wire-format canonical-order checks for IStr-keyed containers.
//!
//! Before the interner removal these checks spawned two child processes to
//! prove the wire bytes did not depend on the *global interner admission
//! order*. With the global pool gone (IStr is now an owned `CompactString`
//! newtype with lexicographic Ord), admission order no longer exists — the only
//! remaining order-sensitivity is *insertion* order into a `PropertyMap`, which
//! is canonicalized to lexicographic key order at serialize time. These are now
//! same-process assertions that two different insertion orders of the same keys
//! produce byte-identical canonical wire.

use selene_core::{PropertyMap, Value, intern};

/// Two `PropertyMap`s built from different insertion orders of the same keys
/// serialize to byte-identical (canonical lexicographic) postcard wire.
#[test]
fn property_map_wire_bytes_are_independent_of_insertion_order() {
    let apple = intern("wire.apple").unwrap();
    let banana = intern("wire.banana").unwrap();
    let zebra = intern("wire.zebra").unwrap();

    let first = PropertyMap::from_pairs([
        (zebra.clone(), Value::Int(3)),
        (apple.clone(), Value::Int(1)),
        (banana.clone(), Value::Int(2)),
    ])
    .expect("first map builds");
    let second = PropertyMap::from_pairs([
        (banana, Value::Int(2)),
        (zebra, Value::Int(3)),
        (apple, Value::Int(1)),
    ])
    .expect("second map builds");

    let first_bytes = postcard::to_allocvec(&first).expect("first serializes");
    let second_bytes = postcard::to_allocvec(&second).expect("second serializes");
    assert_eq!(
        first_bytes, second_bytes,
        "canonical wire order must not depend on PropertyMap insertion order"
    );

    // And both round-trip back to the same map.
    let round: PropertyMap = postcard::from_bytes(&first_bytes).expect("round-trips");
    assert_eq!(round, first);
    assert_eq!(round, second);
}

/// The same canonicalization holds for a Compact-shaped map: a fixed key set
/// supplied in two different orders serializes identically.
#[test]
fn compact_property_map_wire_bytes_are_independent_of_key_order() {
    let alpha = intern("wire.compact.alpha").unwrap();
    let mid = intern("wire.compact.mid").unwrap();
    let omega = intern("wire.compact.omega").unwrap();

    let forward = PropertyMap::compact(
        [alpha.clone(), mid.clone(), omega.clone()],
        [
            Some(Value::Int(1)),
            Some(Value::Int(2)),
            Some(Value::Int(3)),
        ],
    )
    .expect("forward compact builds");
    let reverse = PropertyMap::compact(
        [omega, mid, alpha],
        [
            Some(Value::Int(3)),
            Some(Value::Int(2)),
            Some(Value::Int(1)),
        ],
    )
    .expect("reverse compact builds");

    assert_eq!(
        postcard::to_allocvec(&forward).expect("forward serializes"),
        postcard::to_allocvec(&reverse).expect("reverse serializes"),
        "compact canonical wire must not depend on supplied key order"
    );
}
