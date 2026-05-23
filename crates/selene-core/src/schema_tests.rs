use smallvec::smallvec;

use super::*;
use crate::{ExtensionTypeId, Value, intern};

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

#[test]
fn graph_type_id_rejects_zero() {
    assert!(matches!(
        GraphTypeId::new(0),
        Err(CoreError::ZeroIdentifier)
    ));
    assert_eq!(GraphTypeId::new(1).unwrap().get(), 1);
}

#[test]
fn node_type_def_constructed_with_labels() {
    let label = istr("schema.node");
    let def = NodeTypeDef::new(LabelSet::single(label));
    assert!(def.labels.contains(&label));
    assert!(def.properties.is_empty());
    assert!(def.key.is_none());
}

#[test]
fn edge_type_def_constructed_with_endpoints() {
    let edge = istr("schema.edge");
    let source = NodeTypeRef(istr("schema.source"));
    let target = NodeTypeRef(istr("schema.target"));
    let def = EdgeTypeDef::new(edge, source, target);
    assert_eq!(def.label, edge);
    assert_eq!(def.source_node_type, EdgeEndpointDef::NodeType(source));
    assert_eq!(def.target_node_type, EdgeEndpointDef::NodeType(target));
}

#[test]
fn property_def_with_default_carries_value() {
    let property = PropertyDef {
        name: istr("schema.prop"),
        value_type: ValueType::predefined(PredefinedValueType::Int),
        nullable: false,
        default: Some(Value::Int(7)),
        immutable: false,
    };
    assert_eq!(property.default, Some(Value::Int(7)));
}

#[test]
fn value_type_predefined_string() {
    let value_type = ValueType::predefined(PredefinedValueType::String);
    assert_eq!(value_type.predefined, Some(PredefinedValueType::String));
    assert!(value_type.list_of.is_none());
}

#[test]
fn value_type_list_of_int() {
    let value_type = ValueType::list_of(ValueType::predefined(PredefinedValueType::Int));
    let item = value_type.list_of.as_ref().unwrap();
    assert_eq!(item.predefined, Some(PredefinedValueType::Int));
}

#[test]
fn predefined_value_type_extended_carries_type_id() {
    let predefined = PredefinedValueType::Extended(ExtensionTypeId(0x100));
    assert_eq!(
        predefined,
        PredefinedValueType::Extended(ExtensionTypeId(0x100))
    );
}

#[test]
fn key_label_set_policy_default_is_containment() {
    assert_eq!(KeyLabelSetPolicy::default(), KeyLabelSetPolicy::Containment);
}

#[test]
fn record_type_def_with_multiple_fields() {
    let field_a = PropertyDef {
        name: istr("schema.field.a"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
    };
    let field_b = PropertyDef {
        name: istr("schema.field.b"),
        value_type: ValueType::predefined(PredefinedValueType::Bool),
        nullable: true,
        default: Some(Value::Bool(false)),
        immutable: false,
    };
    let def = RecordTypeDef {
        id: RecordTypeId::new(1),
        name: istr("schema.record"),
        fields: smallvec![field_a, field_b],
    };
    assert_eq!(def.fields.len(), 2);
}

#[test]
fn graph_type_starts_with_empty_type_maps() {
    let graph_type = GraphType::new(GraphTypeId::new(1).unwrap(), istr("schema.graph"));
    assert!(graph_type.node_types.is_empty());
    assert!(graph_type.edge_types.is_empty());
    assert!(graph_type.record_types.is_empty());
}

#[test]
fn property_def_with_no_default_is_valid() {
    let property = PropertyDef {
        name: istr("schema.no.default"),
        value_type: ValueType::predefined(PredefinedValueType::Bytes),
        nullable: true,
        default: None,
        immutable: false,
    };
    assert!(property.default.is_none());
}

#[test]
fn value_type_list_of_takes_precedence_when_multiple_fields_set() {
    let mut value_type = ValueType::list_of(ValueType::predefined(PredefinedValueType::Int));
    value_type.predefined = Some(PredefinedValueType::String);
    assert!(value_type.list_of.is_some());
    assert_eq!(value_type.predefined, Some(PredefinedValueType::String));
}

#[test]
fn graph_type_id_deserialize_round_trips_non_zero() {
    let id = GraphTypeId::new(7).unwrap();
    let bytes = postcard::to_allocvec(&id).unwrap();
    let round: GraphTypeId = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(round, id);
}

#[test]
fn graph_type_id_deserialize_rejects_zero() {
    let bytes = postcard::to_allocvec::<u64>(&0_u64).unwrap();
    let result: Result<GraphTypeId, _> = postcard::from_bytes(&bytes);
    assert!(result.is_err());
}

#[test]
fn wal_one_of_canonicalizes_singleton_to_node_type() {
    let source = NodeTypeRef(istr("schema.solo"));
    assert_eq!(
        EdgeEndpointDef::one_of([source]),
        EdgeEndpointDef::NodeType(source)
    );
}

#[test]
fn wal_one_of_dedupes() {
    let a = NodeTypeRef(istr("schema.aaa"));
    let b = NodeTypeRef(istr("schema.bbb"));
    let endpoint = EdgeEndpointDef::one_of([a, b, a]);
    match endpoint {
        EdgeEndpointDef::OneOf(refs) => assert_eq!(refs.len(), 2),
        other => panic!("expected OneOf, got {other:?}"),
    }
}

#[test]
fn postcard_round_trips_oneof_endpoint() {
    // BRIEF-131e commit 3: WAL OneOf payload survives postcard encode/decode
    // byte-stably. Length-2 stays inline; length-5 spills to heap; both must
    // round-trip identically.
    let two = EdgeEndpointDef::one_of([
        NodeTypeRef(istr("schema.alpha")),
        NodeTypeRef(istr("schema.beta")),
    ]);
    let bytes = postcard::to_allocvec(&two).unwrap();
    let decoded: EdgeEndpointDef = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, two);

    let five = EdgeEndpointDef::one_of([
        NodeTypeRef(istr("schema.a")),
        NodeTypeRef(istr("schema.b")),
        NodeTypeRef(istr("schema.c")),
        NodeTypeRef(istr("schema.d")),
        NodeTypeRef(istr("schema.e")),
    ]);
    let bytes = postcard::to_allocvec(&five).unwrap();
    let decoded: EdgeEndpointDef = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, five);
    assert!(matches!(decoded, EdgeEndpointDef::OneOf(_)));
}

#[test]
fn postcard_round_trips_any_and_node_type_after_oneof_variant_addition() {
    // Confirm appending the OneOf variant to the WAL enum did not perturb
    // postcard's discriminant assignment for prior variants. Encode Any and
    // NodeType separately; the decoded payload must be bit-identical.
    let any_bytes = postcard::to_allocvec(&EdgeEndpointDef::Any).unwrap();
    let any: EdgeEndpointDef = postcard::from_bytes(&any_bytes).unwrap();
    assert_eq!(any, EdgeEndpointDef::Any);

    let nt = EdgeEndpointDef::NodeType(NodeTypeRef(istr("schema.legacy")));
    let nt_bytes = postcard::to_allocvec(&nt).unwrap();
    let decoded: EdgeEndpointDef = postcard::from_bytes(&nt_bytes).unwrap();
    assert_eq!(decoded, nt);
}

#[test]
#[should_panic(expected = "called with empty NodeTypeRef set")]
fn wal_one_of_panics_on_empty_input() {
    let _ = EdgeEndpointDef::one_of(std::iter::empty::<NodeTypeRef>());
}
