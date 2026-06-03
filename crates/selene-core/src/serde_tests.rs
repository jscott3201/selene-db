use std::fmt::Debug;
use std::sync::Arc;

use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use smallvec::smallvec;

use crate::*;

fn rt<T>(value: &T)
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + Debug,
{
    let bytes = postcard::to_allocvec(value)
        .unwrap_or_else(|error| panic!("postcard encode failed for {value:?}: {error:?}"));
    let decoded: T = postcard::from_bytes(&bytes)
        .unwrap_or_else(|error| panic!("postcard decode failed for {value:?}: {error:?}"));
    assert_eq!(&decoded, value);
}

fn istr(value: &str) -> IStr {
    intern(value).unwrap()
}

fn graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).unwrap()
}

fn property_def(name: &str) -> PropertyDef {
    PropertyDef {
        name: istr(name),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        record_fields: None,
    }
}

fn graph_type() -> GraphType {
    let mut graph_type = GraphType::new(graph_type_id(), istr("serde.graph_type"));
    graph_type.node_types.insert(
        istr("serde.node"),
        NodeTypeDef {
            labels: LabelSet::single(istr("serde.node")),
            properties: smallvec![property_def("serde.node.name")],
            key: Some(NodeKey {
                property_names: smallvec![istr("serde.node.name")],
            }),
            validation_mode: ValidationMode::Strict,
        },
    );
    graph_type.edge_types.insert(
        istr("serde.edge"),
        EdgeTypeDef::new(
            istr("serde.edge"),
            NodeTypeRef(istr("serde.node")),
            NodeTypeRef(istr("serde.node")),
        ),
    );
    graph_type.record_types.insert(
        RecordTypeId::new(1),
        RecordTypeDef {
            id: RecordTypeId::new(1),
            name: istr("serde.record"),
            fields: smallvec![property_def("serde.record.field")],
        },
    );
    graph_type
}

fn all_values() -> Vec<Value> {
    let path = Path {
        graph: GraphId::new(1),
        start: NodeId::new(1),
        segments: smallvec![PathSegment {
            edge: EdgeId::new(1),
            direction: EdgeDirection::Outgoing,
            node: NodeId::new(2),
        }],
    };
    vec![
        Value::Bool(true),
        Value::Int(-1),
        Value::Uint(1),
        Value::Int128(-2),
        Value::Uint128(2),
        Value::Float(1.5),
        Value::Float32(2.5),
        Value::Decimal(rust_decimal::Decimal::new(1234, 2)),
        Value::String(istr("serde.value.string")),
        Value::Bytes(Arc::from([1_u8, 2, 3])),
        Value::List(vec![Value::Int(1), Value::Null]),
        Value::Record(Box::new(Record::Open(smallvec![(
            istr("serde.record.open"),
            Value::Bool(false),
        )]))),
        Value::RecordTyped(Box::new(RecordTyped {
            type_id: RecordTypeId::new(1),
            values: smallvec![Some(Value::String(istr("serde.record.typed"))), None],
        })),
        Value::Path(Box::new(path)),
        Value::NodeRef(NodeId::new(1)),
        Value::EdgeRef(EdgeId::new(1)),
        Value::GraphRef(GraphId::new(1)),
        Value::TableRef(BindingTableId::new(1)),
        Value::ZonedDateTime(Box::new(
            "2026-05-07T12:34:56-04:00[America/New_York]"
                .parse()
                .unwrap(),
        )),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
        Value::Date("2026-05-07".parse().unwrap()),
        Value::ZonedTime(Box::new(
            "2026-05-07T12:34:56-04:00[America/New_York]"
                .parse()
                .unwrap(),
        )),
        Value::LocalTime("12:34:56".parse().unwrap()),
        Value::Duration(Box::new("PT1H2S".parse().unwrap())),
        Value::Extended {
            type_id: ExtensionTypeId(0x100),
            payload: Arc::from([1_u8, 2, 3, 4]),
        },
        Value::Null,
        Value::Uuid(uuid::Uuid::nil()),
        Value::Vector(VectorValue::new(vec![1.0, 2.0, 3.0]).unwrap()),
    ]
}

#[test]
fn value_postcard_round_trip() {
    for value in all_values() {
        rt(&value);
    }
}

#[test]
fn property_map_postcard_round_trip() {
    let standard = PropertyMap::from_pairs([
        (istr("serde.pm.a"), Value::Int(1)),
        (istr("serde.pm.b"), Value::Null),
    ])
    .unwrap();
    rt(&standard);

    let compact = PropertyMap::compact(
        [istr("serde.pm.a"), istr("serde.pm.b")],
        [Some(Value::Int(1)), None],
    )
    .unwrap();
    rt(&compact);
}

#[test]
fn label_set_postcard_round_trip() {
    rt(&LabelSet::from_iter([
        istr("serde.label.b"),
        istr("serde.label.a"),
    ]));
}

#[test]
fn change_postcard_round_trip() {
    let label = istr("serde.change.label");
    let property = istr("serde.change.property");
    let changes = vec![
        Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(label.clone()),
            properties: PropertyMap::from_pairs([(property.clone(), Value::Int(1))]).unwrap(),
        },
        Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([istr("serde.change.add")], [istr("serde.change.remove")])
                .unwrap(),
            properties_diff: PropertyDiff::new([(property.clone(), Value::Null)], []).unwrap(),
        },
        Change::NodeDeleted { id: NodeId::new(1) },
        Change::NodePropertyRemoved {
            id: NodeId::new(1),
            property: property.clone(),
        },
        Change::NodeLabelRemoved {
            id: NodeId::new(1),
            label: label.clone(),
        },
        Change::EdgeCreated {
            id: EdgeId::new(1),
            label: label.clone(),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
        Change::EdgeUpdated {
            id: EdgeId::new(1),
            properties_diff: PropertyDiff::new([(property.clone(), Value::Bool(true))], [])
                .unwrap(),
        },
        Change::EdgeDeleted { id: EdgeId::new(1) },
        Change::EdgePropertyRemoved {
            id: EdgeId::new(1),
            property,
        },
        Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::GraphTypeCreated {
                graph_type: graph_type(),
            },
        },
        Change::NodesOfTypeTruncated {
            label: label.clone(),
        },
        Change::EdgesOfTypeTruncated { label },
        Change::GraphReset {},
    ];
    for change in changes {
        rt(&change);
    }
}

#[test]
fn graph_reset_postcard_round_trip() {
    // BRIEF-152: GraphReset is the empty (carries-nothing) factory-reset change.
    // It is the last appended variant, so its postcard tag is 12 (the removal of
    // the former IndexExtensionEvent variant at tag 7 re-tagged every later
    // variant down by one — a greenfield postcard break with no production WAL).
    // A unit-style variant encodes to its single tag byte with no payload.
    let change = Change::GraphReset {};
    let bytes = postcard::to_allocvec(&change).unwrap();
    assert_eq!(
        bytes,
        [12_u8],
        "GraphReset encodes to its bare tag byte (12)"
    );
    let decoded: Change = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, Change::GraphReset {});
}

#[test]
fn pre_147_node_updated_wire_blob_still_decodes() {
    let bytes = [1_u8, 1, 0, 0, 0, 0];
    let decoded: Change = postcard::from_bytes(&bytes).unwrap();

    assert_eq!(
        decoded,
        Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([], []).unwrap(),
            properties_diff: PropertyDiff::new([], []).unwrap(),
        }
    );
}

#[test]
fn schema_change_postcard_round_trip() {
    let graph_type_id = graph_type_id();
    let node_label = istr("serde.schema.node");
    let edge_label = istr("serde.schema.edge");
    let changes = vec![
        SchemaChange::GraphCreated {
            id: GraphId::new(1),
            name: istr("serde.schema.graph"),
            graph_type: Some(graph_type_id),
        },
        SchemaChange::GraphDropped {
            id: GraphId::new(1),
        },
        SchemaChange::GraphTypeCreated {
            graph_type: graph_type(),
        },
        SchemaChange::GraphTypeDropped { id: graph_type_id },
        SchemaChange::NodeTypeAdded {
            graph_type: graph_type_id,
            label: node_label.clone(),
            def: NodeTypeDefV1::new(LabelSet::single(node_label.clone())),
        },
        SchemaChange::EdgeTypeAdded {
            graph_type: graph_type_id,
            label: edge_label.clone(),
            def: EdgeTypeDefV1::new(
                edge_label.clone(),
                NodeTypeRef(node_label.clone()),
                NodeTypeRef(node_label.clone()),
            ),
        },
        SchemaChange::NodeTypeDropped {
            graph_type: graph_type_id,
            name: node_label.clone(),
        },
        SchemaChange::EdgeTypeDropped {
            graph_type: graph_type_id,
            name: edge_label,
        },
        SchemaChange::RecordTypeAdded {
            graph_type: graph_type_id,
            def: RecordTypeDef {
                id: RecordTypeId::new(1),
                name: istr("serde.schema.record"),
                fields: smallvec![property_def("serde.schema.field")],
            },
        },
        SchemaChange::PropertyIndexCreated {
            label: node_label.clone(),
            property: istr("serde.schema.indexed"),
            kind: SchemaPropertyIndexKind::I64,
        },
        SchemaChange::PropertyIndexDropped {
            label: node_label.clone(),
            property: istr("serde.schema.indexed"),
        },
        SchemaChange::PropertyIndexCreatedNamed {
            label: node_label.clone(),
            property: istr("serde.schema.indexed"),
            kind: SchemaPropertyIndexKind::I64,
            name: Some(istr("serde.schema.index.name")),
        },
        SchemaChange::CompositePropertyIndexCreated {
            label: node_label.clone(),
            properties: smallvec![
                istr("serde.schema.indexed.a"),
                istr("serde.schema.indexed.b")
            ],
            kinds: smallvec![
                SchemaPropertyIndexKind::I64,
                SchemaPropertyIndexKind::String
            ],
            name: Some(istr("serde.schema.composite.index.name")),
        },
        SchemaChange::CompositePropertyIndexDropped {
            label: node_label.clone(),
            properties: smallvec![
                istr("serde.schema.indexed.a"),
                istr("serde.schema.indexed.b")
            ],
        },
        SchemaChange::NodeTypeAddedV2 {
            graph_type: graph_type_id,
            label: istr("serde.schema.node.v2"),
            def: NodeTypeDef::new(LabelSet::single(istr("serde.schema.node.v2"))),
        },
        SchemaChange::EdgeTypeAddedV2 {
            graph_type: graph_type_id,
            label: istr("serde.schema.edge.v2"),
            def: EdgeTypeDef::new(
                istr("serde.schema.edge.v2"),
                NodeTypeRef(node_label.clone()),
                NodeTypeRef(node_label),
            ),
        },
    ];
    for change in changes {
        rt(&change);
    }
}

#[test]
fn istr_postcard_round_trips() {
    let value = istr("serde.istr.canonical");
    let bytes = postcard::to_allocvec(&value).unwrap();
    let decoded: IStr = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, intern("serde.istr.canonical").unwrap());
}

#[test]
fn istr_deserialize_from_str_preserves_content() {
    let bytes = postcard::to_allocvec("serde.istr.fresh").unwrap();
    let decoded: IStr = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.as_str(), "serde.istr.fresh");
}

#[test]
fn record_fields_none_open_closed_postcard_round_trip() {
    // CORE-11: the None / Some(Open) / Some(Closed) distinction on
    // PropertyDef.record_fields is load-bearing — WAL replay degrades a bare
    // RECORD to Null if the Open marker does not round-trip. Pin all three
    // durable states at the core level.
    let not_record = PropertyDef {
        name: istr("core-11.not-record"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        record_fields: None,
    };
    rt(&not_record);

    let bare_record = PropertyDef {
        name: istr("core-11.bare-record"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: true,
        default: None,
        immutable: false,
        record_fields: Some(Box::new(RecordFieldStructure::Open)),
    };
    rt(&bare_record);

    let closed_record = PropertyDef {
        name: istr("core-11.closed-record"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        record_fields: Some(Box::new(RecordFieldStructure::Closed(vec![
            RecordFieldStructureDef {
                name: istr("core-11.field.scalar"),
                field_type: RecordFieldStructureType::Scalar(PropertyValueType::Int),
                required: true,
            },
        ]))),
    };
    rt(&closed_record);
}

#[test]
fn nested_record_field_structure_postcard_round_trip() {
    // CORE-11: GV48 nested record types — a closed RECORD whose fields are
    // themselves a LIST<RECORD{..}> and a nested closed RECORD. Exercises the
    // recursive RecordFieldStructureType { Scalar, List, Record } codec.
    let inner_closed = RecordFieldStructure::Closed(vec![RecordFieldStructureDef {
        name: istr("core-11.nested.inner"),
        field_type: RecordFieldStructureType::Scalar(PropertyValueType::Bool),
        required: false,
    }]);

    let structure = RecordFieldStructure::Closed(vec![
        RecordFieldStructureDef {
            name: istr("core-11.nested.list-of-records"),
            field_type: RecordFieldStructureType::List(Box::new(RecordFieldStructureType::Record(
                Box::new(inner_closed.clone()),
            ))),
            required: true,
        },
        RecordFieldStructureDef {
            name: istr("core-11.nested.nested-record"),
            field_type: RecordFieldStructureType::Record(Box::new(RecordFieldStructure::Open)),
            required: false,
        },
        RecordFieldStructureDef {
            name: istr("core-11.nested.nested-closed"),
            field_type: RecordFieldStructureType::Record(Box::new(inner_closed)),
            required: true,
        },
    ]);
    rt(&structure);

    // And the same structure carried on a PropertyDef.record_fields slot.
    let def = PropertyDef {
        name: istr("core-11.nested.def"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        record_fields: Some(Box::new(structure)),
    };
    rt(&def);
}

#[test]
fn extended_value_payload_postcard_round_trip() {
    let value = Value::Extended {
        type_id: ExtensionTypeId(0x100),
        payload: Arc::from([1_u8, 2, 3, 4]),
    };
    rt(&value);
}

#[test]
fn vector_value_postcard_rejects_invalid_payloads() {
    let empty = postcard::to_allocvec(&Vec::<f32>::new()).unwrap();
    assert!(postcard::from_bytes::<VectorValue>(&empty).is_err());

    let non_finite = postcard::to_allocvec(&vec![1.0_f32, f32::INFINITY]).unwrap();
    assert!(postcard::from_bytes::<VectorValue>(&non_finite).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn simple_values_postcard_round_trip(value in simple_value_strategy()) {
        rt(&value);
    }
}

fn simple_value_strategy() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        any::<u64>().prop_map(Value::Uint),
        any::<f64>()
            .prop_filter("finite", |value| value.is_finite())
            .prop_map(Value::Float),
        any::<f32>()
            .prop_filter("finite", |value| value.is_finite())
            .prop_map(Value::Float32),
        (0_u8..32).prop_map(|idx| {
            let name = format!("serde.prop.string.{idx}");
            Value::String(intern(&name).unwrap())
        }),
        proptest::collection::vec(any::<u8>(), 0..32)
            .prop_map(|bytes| Value::Bytes(Arc::from(bytes))),
        proptest::collection::vec(
            any::<f32>().prop_filter("finite", |value| value.is_finite()),
            1..32,
        )
        .prop_map(|components| Value::Vector(VectorValue::new(components).unwrap())),
        any::<u128>().prop_map(Value::Uint128),
        any::<i128>().prop_map(Value::Int128),
        Just(Value::Null),
        any::<u128>().prop_map(|raw| Value::Uuid(uuid::Uuid::from_u128(raw))),
    ]
}
