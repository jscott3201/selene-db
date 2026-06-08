//! WAL-replay recovery round-trip coverage for closed/typed and open `RECORD` property
//! declarations (JSON/L1c-d). The rkyv snapshot side is pinned by
//! `core_provider::sections::gtyp::gtyp_round_trips_record_field_types`; these tests pin
//! the *serde/WAL* type-model (the second of the dual type-models, D14/D19) which the
//! snapshot test does not exercise. The open-record case is a regression pin for the
//! recovery bug where a bare `RECORD` property degraded to `Null` on replay.

use std::fs;

use selene_core::{
    Change, GraphId, GraphTypeId, LabelSet, PredefinedValueType, PropertyValueType,
    RecordFieldStructure, RecordFieldStructureDef, RecordFieldStructureType, SchemaChange,
    ValueType, db_string,
};
use smallvec::smallvec;

use crate::{
    MAX_RECORD_TYPE_NESTING, PropertyTypeDef, RecordFieldType, RecordFieldTypeDef,
    RecordFieldTypes, SharedGraph, ValidationMode,
};

use super::{append_wal, empty_closed_graph_type, temp_dir};

/// A nested record descriptor:
/// `RECORD{a :: INT, b :: LIST<STRING>, c :: RECORD{d :: BOOL}, meta :: RECORD}`.
fn nested_record_field_types() -> RecordFieldTypes {
    RecordFieldTypes(vec![
        RecordFieldTypeDef {
            name: db_string("a").unwrap(),
            field_type: RecordFieldType::Scalar(PropertyValueType::Int),
            required: true,
        },
        RecordFieldTypeDef {
            name: db_string("b").unwrap(),
            field_type: RecordFieldType::List(Box::new(RecordFieldType::Scalar(
                PropertyValueType::String,
            ))),
            required: false,
        },
        RecordFieldTypeDef {
            name: db_string("c").unwrap(),
            field_type: RecordFieldType::Record(Box::new(RecordFieldTypes(vec![
                RecordFieldTypeDef {
                    name: db_string("d").unwrap(),
                    field_type: RecordFieldType::Scalar(PropertyValueType::Bool),
                    required: true,
                },
            ]))),
            required: true,
        },
        RecordFieldTypeDef {
            name: db_string("meta").unwrap(),
            field_type: RecordFieldType::OpenRecord,
            required: false,
        },
    ])
}

#[test]
fn recover_closed_wal_only_preserves_closed_record_property() {
    let dir = temp_dir("closed-schema-record-wal-only");
    let graph_id = GraphId::new(31);
    let base = empty_closed_graph_type();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let sensor = db_string("RecordSensor").unwrap();
    let config = db_string("config").unwrap();
    let field_types = nested_record_field_types();
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node_type(
                sensor.clone(),
                LabelSet::single(sensor),
                vec![PropertyTypeDef {
                    name: config.clone(),
                    value_type: PropertyValueType::RecordTyped,
                    list_element_type: None,
                    required: false,
                    default: None,
                    immutable: false,
                    unique: false,
                    record_field_types: Some(field_types.clone()),
                }],
                ValidationMode::Strict,
            )
            .unwrap();
        txn.commit().unwrap().changes
    };
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    let property = &graph_type.node_types[0].properties[0];
    assert_eq!(property.name, config);
    assert_eq!(property.value_type, PropertyValueType::RecordTyped);
    // Full structural equality pins that the nested LIST + open/closed RECORD field types
    // survived the serde encode → WAL → serde decode round-trip intact.
    assert_eq!(property.record_field_types, Some(field_types));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_closed_wal_only_preserves_open_record_property() {
    // Regression pin: a bare/open `RECORD` property carries no field list, so the only
    // durable record-ness signal is `record_fields == Some(Open)`. Before that marker
    // existed, WAL replay degraded the property to `PropertyValueType::Null`, which then
    // wrongly rejected every record value (G2000) on the recovered graph.
    let dir = temp_dir("closed-schema-open-record-wal-only");
    let graph_id = GraphId::new(32);
    let base = empty_closed_graph_type();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let sensor = db_string("OpenRecordSensor").unwrap();
    let payload = db_string("payload").unwrap();
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node_type(
                sensor.clone(),
                LabelSet::single(sensor),
                vec![PropertyTypeDef {
                    name: payload.clone(),
                    value_type: PropertyValueType::RecordTyped,
                    list_element_type: None,
                    required: false,
                    default: None,
                    immutable: false,
                    unique: false,
                    record_field_types: None,
                }],
                ValidationMode::Strict,
            )
            .unwrap();
        txn.commit().unwrap().changes
    };
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    let property = &graph_type.node_types[0].properties[0];
    assert_eq!(property.name, payload);
    assert_eq!(property.value_type, PropertyValueType::RecordTyped);
    assert_eq!(property.record_field_types, None);
    let _ = fs::remove_dir_all(dir);
}

/// Build an over-deep serde record descriptor: `levels` nested closed RECORDs wrapping a
/// scalar leaf. Hand-built so it bypasses the commit-time encode guard and exercises the
/// recovery-side depth guard directly.
fn deep_record_structure(levels: u32) -> RecordFieldStructure {
    let mut structure = RecordFieldStructure::Closed(vec![RecordFieldStructureDef {
        name: db_string("leaf").unwrap(),
        field_type: RecordFieldStructureType::Scalar(PropertyValueType::Bool),
        required: true,
    }]);
    for _ in 0..levels {
        structure = RecordFieldStructure::Closed(vec![RecordFieldStructureDef {
            name: db_string("nest").unwrap(),
            field_type: RecordFieldStructureType::Record(Box::new(structure)),
            required: true,
        }]);
    }
    structure
}

#[test]
fn recover_closed_rejects_overdeep_record_property() {
    let dir = temp_dir("closed-schema-record-depth");
    let graph_id = GraphId::new(33);
    let base = empty_closed_graph_type();
    let graph_type = GraphTypeId::new(1).unwrap();
    let sensor = db_string("DeepRecordSensor").unwrap();
    // value_type is unread on the record-recovery path (record_fields drives it); a scalar
    // placeholder keeps the hand-built WAL entry well-formed.
    let value_type = ValueType::predefined(PredefinedValueType::String);
    let record_fields = Some(Box::new(deep_record_structure(MAX_RECORD_TYPE_NESTING + 1)));
    append_wal(
        &dir,
        0,
        &[Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::NodeTypeAddedV2 {
                graph_type,
                label: sensor.clone(),
                def: selene_core::NodeTypeDef {
                    labels: LabelSet::single(sensor),
                    properties: smallvec![selene_core::PropertyDef {
                        name: db_string("too_deep").unwrap(),
                        value_type,
                        nullable: true,
                        default: None,
                        immutable: false,
                        unique: false,
                        record_fields,
                    }],
                    key: None,
                    validation_mode: selene_core::ValidationMode::Strict,
                },
            },
        }],
    );

    let err = match SharedGraph::recover_closed(&dir, graph_id, base) {
        Ok(_) => panic!("overdeep RECORD property recovery should fail"),
        Err(err) => err,
    };
    assert!(format!("{err}").contains("RECORD nesting limit"));
    let _ = fs::remove_dir_all(dir);
}
