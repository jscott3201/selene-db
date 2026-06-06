use selene_core::{DbString, Record, Value, VectorValue, db_string};
use smallvec::smallvec;

use super::*;

fn label(name: &str) -> DbString {
    db_string(name).unwrap()
}

fn list_default(items: Vec<PropertyDefaultValue>) -> PropertyDefaultValue {
    PropertyDefaultValue::List(items.into_iter().map(Box::new).collect())
}

fn record_default(fields: Vec<(&str, PropertyDefaultValue)>) -> PropertyDefaultValue {
    PropertyDefaultValue::Record(
        fields
            .into_iter()
            .map(|(name, value)| PropertyDefaultRecordField {
                name: label(name),
                value: Box::new(value),
            })
            .collect(),
    )
}

#[test]
fn property_default_record_descriptors_materialize_nested_values() {
    let value = Value::Record(Box::new(Record::Open(smallvec![
        (label("kind"), Value::String(label("agent"))),
        (
            label("counts"),
            Value::List(vec![Value::Int(1), Value::Int(2)])
        ),
        (
            label("embedding"),
            Value::Vector(VectorValue::new(vec![1.0, 0.0]).unwrap())
        ),
    ])));
    let expected = record_default(vec![
        ("kind", PropertyDefaultValue::String(label("agent"))),
        (
            "counts",
            list_default(vec![
                PropertyDefaultValue::Integer(1),
                PropertyDefaultValue::Integer(2),
            ]),
        ),
        (
            "embedding",
            PropertyDefaultValue::Vector(vec![1.0_f32.to_bits(), 0.0_f32.to_bits()]),
        ),
    ]);

    assert_eq!(
        PropertyDefaultValue::from_value(&value),
        Some(expected.clone())
    );
    assert_eq!(expected.to_value().unwrap(), value);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&expected).unwrap();
    let decoded = rkyv::from_bytes::<PropertyDefaultValue, rkyv::rancor::Error>(&bytes).unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn property_default_record_descriptors_reject_duplicate_fields() {
    let value = Value::Record(Box::new(Record::Open(smallvec![
        (label("kind"), Value::String(label("agent"))),
        (label("kind"), Value::String(label("duplicate"))),
    ])));
    assert!(PropertyDefaultValue::from_value(&value).is_none());

    let duplicate = record_default(vec![
        ("kind", PropertyDefaultValue::String(label("agent"))),
        ("kind", PropertyDefaultValue::String(label("duplicate"))),
    ]);
    assert!(matches!(
        duplicate.to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("RECORD property default contains duplicate field kind")
    ));
}
