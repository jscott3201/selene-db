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

fn dbs(value: &str) -> DbString {
    crate::db_string(value).unwrap()
}

fn graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).unwrap()
}

fn property_def(name: &str) -> PropertyDef {
    PropertyDef {
        name: dbs(name),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        unique: false,
        record_fields: None,
    }
}

fn graph_type() -> GraphType {
    let mut graph_type = GraphType::new(graph_type_id(), dbs("serde.graph_type"));
    graph_type.node_types.insert(
        dbs("serde.node"),
        NodeTypeDef {
            labels: LabelSet::single(dbs("serde.node")),
            properties: smallvec![property_def("serde.node.name")],
            key: Some(NodeKey {
                property_names: smallvec![dbs("serde.node.name")],
            }),
            validation_mode: ValidationMode::Strict,
        },
    );
    graph_type.edge_types.insert(
        dbs("serde.edge"),
        EdgeTypeDef::new(
            dbs("serde.edge"),
            NodeTypeRef(dbs("serde.node")),
            NodeTypeRef(dbs("serde.node")),
        ),
    );
    graph_type.record_types.insert(
        RecordTypeId::new(1),
        RecordTypeDef {
            id: RecordTypeId::new(1),
            name: dbs("serde.record"),
            fields: smallvec![property_def("serde.record.field")],
        },
    );
    graph_type
}

#[test]
fn property_def_unique_flag_postcard_round_trip() {
    let mut property = property_def("serde.unique");
    property.unique = true;
    rt(&property);
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
        Value::String(dbs("serde.value.string")),
        Value::Bytes(Arc::from([1_u8, 2, 3])),
        Value::List(vec![Value::Int(1), Value::Null]),
        Value::Record(Box::new(Record::Open(smallvec![(
            dbs("serde.record.open"),
            Value::Bool(false),
        )]))),
        Value::RecordTyped(Box::new(RecordTyped {
            type_id: RecordTypeId::new(1),
            values: smallvec![Some(Value::String(dbs("serde.record.typed"))), None],
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
        Value::Json(JsonValue::new(serde_json::json!({"serde": ["json", 1]})).unwrap()),
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
        (dbs("serde.pm.a"), Value::Int(1)),
        (dbs("serde.pm.b"), Value::Null),
    ])
    .unwrap();
    rt(&standard);

    let compact = PropertyMap::compact(
        [dbs("serde.pm.a"), dbs("serde.pm.b")],
        [Some(Value::Int(1)), None],
    )
    .unwrap();
    rt(&compact);
}

#[test]
fn label_set_postcard_round_trip() {
    rt(&LabelSet::from_iter([
        dbs("serde.label.b"),
        dbs("serde.label.a"),
    ]));
}

#[test]
fn change_postcard_round_trip() {
    let label = dbs("serde.change.label");
    let property = dbs("serde.change.property");
    let changes = vec![
        Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(label.clone()),
            properties: PropertyMap::from_pairs([(property.clone(), Value::Int(1))]).unwrap(),
        },
        Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([dbs("serde.change.add")], [dbs("serde.change.remove")])
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
    let node_label = dbs("serde.schema.node");
    let edge_label = dbs("serde.schema.edge");
    let changes = vec![
        SchemaChange::GraphCreated {
            id: GraphId::new(1),
            name: dbs("serde.schema.graph"),
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
                name: dbs("serde.schema.record"),
                fields: smallvec![property_def("serde.schema.field")],
            },
        },
        SchemaChange::PropertyIndexCreated {
            label: node_label.clone(),
            property: dbs("serde.schema.indexed"),
            kind: SchemaPropertyIndexKind::Bool,
        },
        SchemaChange::PropertyIndexDropped {
            label: node_label.clone(),
            property: dbs("serde.schema.indexed"),
        },
        SchemaChange::PropertyIndexCreatedNamed {
            label: node_label.clone(),
            property: dbs("serde.schema.indexed"),
            kind: SchemaPropertyIndexKind::Decimal,
            name: Some(dbs("serde.schema.index.name")),
        },
        SchemaChange::CompositePropertyIndexCreated {
            label: node_label.clone(),
            properties: smallvec![
                dbs("serde.schema.indexed.a"),
                dbs("serde.schema.indexed.b"),
                dbs("serde.schema.indexed.c"),
                dbs("serde.schema.indexed.d"),
                dbs("serde.schema.indexed.e"),
                dbs("serde.schema.indexed.f"),
                dbs("serde.schema.indexed.g"),
                dbs("serde.schema.indexed.h"),
            ],
            kinds: smallvec![
                SchemaPropertyIndexKind::U64,
                SchemaPropertyIndexKind::I128,
                SchemaPropertyIndexKind::U128,
                SchemaPropertyIndexKind::Decimal,
                SchemaPropertyIndexKind::ZonedDateTime,
                SchemaPropertyIndexKind::LocalTime,
                SchemaPropertyIndexKind::ZonedTime,
                SchemaPropertyIndexKind::Duration,
            ],
            name: Some(dbs("serde.schema.composite.index.name")),
        },
        SchemaChange::CompositePropertyIndexDropped {
            label: node_label.clone(),
            properties: smallvec![
                dbs("serde.schema.indexed.a"),
                dbs("serde.schema.indexed.b"),
                dbs("serde.schema.indexed.c"),
                dbs("serde.schema.indexed.d"),
            ],
        },
        SchemaChange::VectorIndexCreated {
            label: node_label.clone(),
            property: dbs("serde.schema.embedding"),
            kind: SchemaVectorIndexKind::Flat,
            dimension: 3,
            name: Some(dbs("serde.schema.vector.index.name")),
            hnsw_config: None,
            ivf_config: None,
        },
        SchemaChange::VectorIndexCreated {
            label: node_label.clone(),
            property: dbs("serde.schema.hnsw.embedding"),
            kind: SchemaVectorIndexKind::HnswCosine,
            dimension: 3,
            name: Some(dbs("serde.schema.vector.hnsw.index.name")),
            hnsw_config: Some(crate::HnswIndexConfig::new(24, 128)),
            ivf_config: None,
        },
        SchemaChange::VectorIndexCreated {
            label: node_label.clone(),
            property: dbs("serde.schema.ivf.embedding"),
            kind: SchemaVectorIndexKind::IvfCosine,
            dimension: 3,
            name: Some(dbs("serde.schema.vector.ivf.index.name")),
            hnsw_config: None,
            ivf_config: Some(crate::IvfIndexConfig::new(128)),
        },
        SchemaChange::VectorIndexCreated {
            label: node_label.clone(),
            property: dbs("serde.schema.turbo.embedding"),
            kind: SchemaVectorIndexKind::TurboQuantCosine,
            dimension: 3,
            name: Some(dbs("serde.schema.vector.turbo.index.name")),
            hnsw_config: None,
            ivf_config: None,
        },
        SchemaChange::VectorIndexDropped {
            label: node_label.clone(),
            property: dbs("serde.schema.embedding"),
        },
        SchemaChange::TextIndexCreated {
            label: node_label.clone(),
            property: dbs("serde.schema.body"),
            name: Some(dbs("serde.schema.text.index.name")),
        },
        SchemaChange::TextIndexDropped {
            label: node_label.clone(),
            property: dbs("serde.schema.body"),
        },
        SchemaChange::NodeTypeAddedV2 {
            graph_type: graph_type_id,
            label: dbs("serde.schema.node.v2"),
            def: NodeTypeDef::new(LabelSet::single(dbs("serde.schema.node.v2"))),
        },
        SchemaChange::EdgeTypeAddedV2 {
            graph_type: graph_type_id,
            label: dbs("serde.schema.edge.v2"),
            def: EdgeTypeDef::new(
                dbs("serde.schema.edge.v2"),
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
fn db_string_postcard_round_trips() {
    let value = dbs("serde.db_string.canonical");
    let bytes = postcard::to_allocvec(&value).unwrap();
    let decoded: DbString = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.as_str(), "serde.db_string.canonical");
}

#[test]
fn db_string_deserialize_from_str_preserves_content() {
    let bytes = postcard::to_allocvec("serde.db_string.fresh").unwrap();
    let decoded: DbString = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.as_str(), "serde.db_string.fresh");
}

#[test]
fn record_fields_none_open_closed_postcard_round_trip() {
    // CORE-11: the None / Some(Open) / Some(Closed) distinction on
    // PropertyDef.record_fields is load-bearing — WAL replay degrades a bare
    // RECORD to Null if the Open marker does not round-trip. Pin all three
    // durable states at the core level.
    let not_record = PropertyDef {
        name: dbs("core-11.not-record"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        unique: false,
        record_fields: None,
    };
    rt(&not_record);

    let bare_record = PropertyDef {
        name: dbs("core-11.bare-record"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: true,
        default: None,
        immutable: false,
        unique: false,
        record_fields: Some(Box::new(RecordFieldStructure::Open)),
    };
    rt(&bare_record);

    let closed_record = PropertyDef {
        name: dbs("core-11.closed-record"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        unique: false,
        record_fields: Some(Box::new(RecordFieldStructure::Closed(vec![
            RecordFieldStructureDef {
                name: dbs("core-11.field.scalar"),
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
    // themselves a LIST<RECORD{..} NOT NULL> and a nested closed RECORD.
    // Exercises the recursive RecordFieldStructureType { Scalar, List, Record,
    // NotNull } codec.
    let inner_closed = RecordFieldStructure::Closed(vec![RecordFieldStructureDef {
        name: dbs("core-11.nested.inner"),
        field_type: RecordFieldStructureType::Scalar(PropertyValueType::Bool),
        required: false,
    }]);

    let structure = RecordFieldStructure::Closed(vec![
        RecordFieldStructureDef {
            name: dbs("core-11.nested.list-of-records"),
            field_type: RecordFieldStructureType::List(Box::new(
                RecordFieldStructureType::NotNull(Box::new(RecordFieldStructureType::Record(
                    Box::new(inner_closed.clone()),
                ))),
            )),
            required: true,
        },
        RecordFieldStructureDef {
            name: dbs("core-11.nested.nested-record"),
            field_type: RecordFieldStructureType::Record(Box::new(RecordFieldStructure::Open)),
            required: false,
        },
        RecordFieldStructureDef {
            name: dbs("core-11.nested.nested-closed"),
            field_type: RecordFieldStructureType::Record(Box::new(inner_closed)),
            required: true,
        },
    ]);
    rt(&structure);

    // And the same structure carried on a PropertyDef.record_fields slot.
    let def = PropertyDef {
        name: dbs("core-11.nested.def"),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        unique: false,
        record_fields: Some(Box::new(structure)),
    };
    rt(&def);
}

#[test]
fn decimal_type_metadata_postcard_round_trip() {
    let decimal_type = DecimalType::new(5, 2).expect("valid decimal type");
    let value_type = ValueType {
        predefined: Some(PredefinedValueType::Decimal),
        decimal_type: Some(decimal_type),
        character_string_type: None,
        byte_string_type: None,
        union: None,
        list_of: None,
        record: None,
        not_null: false,
        cardinality: ValueTypeCardinality::ExactlyOne,
    };
    rt(&value_type);

    let structure = RecordFieldStructureType::Decimal(decimal_type);
    rt(&structure);

    let def = PropertyDef {
        name: dbs("core.decimal.typed"),
        value_type,
        nullable: false,
        default: Some(Value::Decimal("123.45".parse().unwrap())),
        immutable: false,
        unique: false,
        record_fields: Some(Box::new(RecordFieldStructure::Closed(vec![
            RecordFieldStructureDef {
                name: dbs("core.decimal.field"),
                field_type: structure,
                required: true,
            },
        ]))),
    };
    rt(&def);
}

#[test]
fn character_string_type_metadata_postcard_round_trip() {
    let character_string_type =
        CharacterStringType::new(2, 4).expect("valid character-string type");
    let value_type = ValueType {
        predefined: Some(PredefinedValueType::String),
        decimal_type: None,
        character_string_type: Some(character_string_type),
        byte_string_type: None,
        union: None,
        list_of: None,
        record: None,
        not_null: false,
        cardinality: ValueTypeCardinality::ExactlyOne,
    };
    rt(&value_type);

    let structure = RecordFieldStructureType::CharacterString(character_string_type);
    rt(&structure);

    let def = PropertyDef {
        name: dbs("core.string.typed"),
        value_type,
        nullable: false,
        default: Some(Value::String(dbs("core"))),
        immutable: false,
        unique: false,
        record_fields: Some(Box::new(RecordFieldStructure::Closed(vec![
            RecordFieldStructureDef {
                name: dbs("core.string.field"),
                field_type: structure,
                required: true,
            },
        ]))),
    };
    rt(&def);
}

#[test]
fn byte_string_type_metadata_postcard_round_trip() {
    let byte_string_type = ByteStringType::new(2, 4).expect("valid byte-string type");
    let value_type = ValueType {
        predefined: Some(PredefinedValueType::Bytes),
        decimal_type: None,
        character_string_type: None,
        byte_string_type: Some(byte_string_type),
        union: None,
        list_of: None,
        record: None,
        not_null: false,
        cardinality: ValueTypeCardinality::ExactlyOne,
    };
    rt(&value_type);

    let structure = RecordFieldStructureType::ByteString(byte_string_type);
    rt(&structure);

    let def = PropertyDef {
        name: dbs("core.bytes.typed"),
        value_type,
        nullable: false,
        default: Some(Value::Bytes(Arc::from([0xCA, 0xFE]))),
        immutable: false,
        unique: false,
        record_fields: Some(Box::new(RecordFieldStructure::Closed(vec![
            RecordFieldStructureDef {
                name: dbs("core.bytes.field"),
                field_type: structure,
                required: true,
            },
        ]))),
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
            Value::String(dbs(&name))
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
