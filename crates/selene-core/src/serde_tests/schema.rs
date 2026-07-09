use std::sync::Arc;

use smallvec::smallvec;

use super::{dbs, graph_type, graph_type_id, property_def, rt};
use crate::*;

#[test]
fn property_def_unique_flag_postcard_round_trip() {
    let mut property = property_def("serde.unique");
    property.unique = true;
    rt(&property);
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
            name: edge_label.clone(),
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
        SchemaChange::EdgePropertyIndexCreated {
            label: edge_label.clone(),
            property: dbs("serde.schema.edge.indexed"),
            kind: SchemaPropertyIndexKind::String,
            name: Some(dbs("serde.schema.edge.index.name")),
        },
        SchemaChange::EdgePropertyIndexDropped {
            label: edge_label.clone(),
            property: dbs("serde.schema.edge.indexed"),
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
        SchemaChange::NodeTypeAlteredV2 {
            graph_type: graph_type_id,
            label: dbs("serde.schema.node.altered.v2"),
            properties: smallvec![{
                let mut property = property_def("serde.schema.node.altered.property");
                property.nullable = true;
                property
            }],
        },
    ];
    for change in changes {
        rt(&change);
    }
}

#[test]
fn node_type_altered_v2_appends_schema_change_postcard_tag() {
    let prior = SchemaChange::EdgePropertyIndexDropped {
        label: dbs("serde.schema.edge"),
        property: dbs("serde.schema.edge.property"),
    };
    let appended = SchemaChange::NodeTypeAlteredV2 {
        graph_type: graph_type_id(),
        label: dbs("serde.schema.node.altered.v2"),
        properties: smallvec![],
    };

    assert_eq!(postcard::to_allocvec(&prior).unwrap()[0], 21);
    assert_eq!(postcard::to_allocvec(&appended).unwrap()[0], 22);
}

#[test]
fn record_fields_none_open_closed_postcard_round_trip() {
    // CORE-11: the None / Some(Open) / Some(Closed) distinction on
    // PropertyDef.record_fields is load-bearing - WAL replay degrades a bare
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
    // CORE-11: GV48 nested record types - a closed RECORD whose fields are
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
