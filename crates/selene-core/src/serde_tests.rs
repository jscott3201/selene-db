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
        Value::ExternalString(Arc::from("serde.value.external-string")),
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
        Value::Path(path),
        Value::NodeRef(NodeId::new(1)),
        Value::EdgeRef(EdgeId::new(1)),
        Value::GraphRef(GraphId::new(1)),
        Value::TableRef(BindingTableId::new(1)),
        Value::ZonedDateTime(
            "2026-05-07T12:34:56-04:00[America/New_York]"
                .parse()
                .unwrap(),
        ),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
        Value::Date("2026-05-07".parse().unwrap()),
        Value::ZonedTime(
            "2026-05-07T12:34:56-04:00[America/New_York]"
                .parse()
                .unwrap(),
        ),
        Value::LocalTime("12:34:56".parse().unwrap()),
        Value::Duration("PT1H2S".parse().unwrap()),
        Value::Extended {
            type_id: ExtensionTypeId(0x100),
            payload: Arc::from([1_u8, 2, 3, 4]),
        },
        Value::Null,
        Value::Uuid(uuid::Uuid::nil()),
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
            labels: LabelSet::single(label),
            properties: PropertyMap::from_pairs([(property, Value::Int(1))]).unwrap(),
        },
        Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([istr("serde.change.add")], [istr("serde.change.remove")])
                .unwrap(),
            properties_diff: PropertyDiff::new([(property, Value::Null)], []).unwrap(),
        },
        Change::NodeDeleted { id: NodeId::new(1) },
        Change::EdgeCreated {
            id: EdgeId::new(1),
            label,
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
        Change::EdgeUpdated {
            id: EdgeId::new(1),
            properties_diff: PropertyDiff::new([(property, Value::Bool(true))], []).unwrap(),
        },
        Change::EdgeDeleted { id: EdgeId::new(1) },
        Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::GraphTypeCreated {
                graph_type: graph_type(),
            },
        },
        Change::IndexExtensionEvent {
            provider: istr("serde.provider"),
            payload: Arc::from([1_u8, 2, 3]),
        },
    ];
    for change in changes {
        rt(&change);
    }
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
            label: node_label,
            def: NodeTypeDefV1::new(LabelSet::single(node_label)),
        },
        SchemaChange::EdgeTypeAdded {
            graph_type: graph_type_id,
            label: edge_label,
            def: EdgeTypeDefV1::new(edge_label, NodeTypeRef(node_label), NodeTypeRef(node_label)),
        },
        SchemaChange::NodeTypeDropped {
            graph_type: graph_type_id,
            name: node_label,
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
        // Reserved discriminants: kept for postcard ABI stability so future
        // additions never re-number `PropertyIndex*` or
        // `ProcedurePackLifecycle`.
        SchemaChange::ProcedurePackActivated {
            pack_name: istr("serde.pack"),
            version: istr("0.0.0"),
        },
        SchemaChange::ProcedurePackDeprecated {
            pack_name: istr("serde.pack"),
            version: istr("0.0.0"),
            reason: istr("serde.reason"),
        },
        SchemaChange::ProcedurePackDisabled {
            pack_name: istr("serde.pack"),
            version: istr("0.0.0"),
            reason: istr("serde.reason"),
        },
        SchemaChange::PropertyIndexCreated {
            label: node_label,
            property: istr("serde.schema.indexed"),
            kind: SchemaPropertyIndexKind::I64,
        },
        SchemaChange::PropertyIndexDropped {
            label: node_label,
            property: istr("serde.schema.indexed"),
        },
        SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::ValidationFailed {
                pack_name: Some(istr("serde.pack")),
                principal: istr("serde.principal"),
                error: "serde.error".to_owned(),
                at: jiff::Timestamp::new(1, 0).unwrap(),
            },
        },
        SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Staged {
                pack_name: istr("serde.pack"),
                content_hash: [1_u8; 32],
                principal: istr("serde.principal"),
                at: jiff::Timestamp::new(2, 0).unwrap(),
            },
        },
        SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Activated {
                pack_name: istr("serde.pack"),
                content_hash: [2_u8; 32],
                principal: istr("serde.principal"),
                at: jiff::Timestamp::new(3, 0).unwrap(),
            },
        },
        SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Deprecated {
                pack_name: istr("serde.pack"),
                reason: istr("serde.reason"),
                principal: istr("serde.principal"),
                at: jiff::Timestamp::new(4, 0).unwrap(),
            },
        },
        SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Disabled {
                pack_name: istr("serde.pack"),
                principal: istr("serde.principal"),
                at: jiff::Timestamp::new(5, 0).unwrap(),
            },
        },
        SchemaChange::PropertyIndexCreatedNamed {
            label: node_label,
            property: istr("serde.schema.indexed"),
            kind: SchemaPropertyIndexKind::I64,
            name: Some(istr("serde.schema.index.name")),
        },
        SchemaChange::CompositePropertyIndexCreated {
            label: node_label,
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
            label: node_label,
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
                NodeTypeRef(node_label),
                NodeTypeRef(node_label),
            ),
        },
    ];
    for change in changes {
        rt(&change);
    }
}

#[test]
fn istr_postcard_round_trips_through_intern_pool() {
    let value = istr("serde.istr.canonical");
    let bytes = postcard::to_allocvec(&value).unwrap();
    let decoded: IStr = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, intern("serde.istr.canonical").unwrap());
}

#[test]
fn istr_deserialize_admits_to_interner() {
    let bytes = postcard::to_allocvec("serde.istr.fresh").unwrap();
    let decoded: IStr = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.as_str(), "serde.istr.fresh");
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
#[ignore = "requires a resettable or reduced-cap test interner; production interner is process-global"]
fn istr_deserialize_respects_cap() {
    let _ = postcard::to_allocvec("would-exceed-cap").unwrap();
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
        any::<u128>().prop_map(Value::Uint128),
        any::<i128>().prop_map(Value::Int128),
        Just(Value::Null),
        any::<u128>().prop_map(|raw| Value::Uuid(uuid::Uuid::from_u128(raw))),
    ]
}
