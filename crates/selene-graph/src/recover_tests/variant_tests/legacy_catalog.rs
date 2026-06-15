use std::fs;

use selene_core::{
    Change, EdgeTypeDefV1, GraphId, GraphTypeId, LabelSet, NodeTypeDefV1, NodeTypeRef,
    PredefinedValueType, PropertyDefV1, SchemaChange, ValueType, ValueTypeCardinality, db_string,
};
use smallvec::smallvec;

use crate::{EdgeEndpointDef, SharedGraph, ValidationMode};

use super::super::{append_wal, empty_closed_graph_type, temp_dir};

fn legacy_string_property(name: &str, required: bool) -> PropertyDefV1 {
    PropertyDefV1 {
        name: db_string(name).unwrap(),
        value_type: ValueType {
            predefined: Some(PredefinedValueType::String),
            decimal_type: None,
            character_string_type: None,
            byte_string_type: None,
            union: None,
            list_of: None,
            record: None,
            not_null: required,
            cardinality: ValueTypeCardinality::ExactlyOne,
        },
        nullable: !required,
        default: None,
    }
}

#[test]
fn recover_closed_wal_only_decodes_legacy_catalog_ddl_v1() {
    let dir = temp_dir("closed-schema-legacy-wal-only");
    let graph_id = GraphId::new(20);
    let base = empty_closed_graph_type();
    let graph_type = GraphTypeId::new(1).unwrap();
    let sensor = db_string("LegacySensor").unwrap();
    let linked = db_string("LEGACY_LINKED").unwrap();
    let changes = vec![
        Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::NodeTypeAdded {
                graph_type,
                label: sensor.clone(),
                def: NodeTypeDefV1 {
                    labels: LabelSet::single(sensor.clone()),
                    properties: smallvec![legacy_string_property("serial", true)],
                    key: None,
                },
            },
        },
        Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::EdgeTypeAdded {
                graph_type,
                label: linked.clone(),
                def: EdgeTypeDefV1 {
                    label: linked.clone(),
                    source_node_type: NodeTypeRef(sensor.clone()),
                    target_node_type: NodeTypeRef(sensor.clone()),
                    properties: smallvec![legacy_string_property("since", false)],
                },
            },
        },
    ];
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    let node_type = &graph_type.node_types[0];
    assert_eq!(node_type.name, sensor);
    assert_eq!(node_type.validation_mode, ValidationMode::Strict);
    assert_eq!(node_type.properties[0].name.as_str(), "serial");
    assert!(node_type.properties[0].required);
    assert!(!node_type.properties[0].immutable);
    let edge_type = &graph_type.edge_types[0];
    assert_eq!(edge_type.name, linked);
    assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(0));
    assert_eq!(edge_type.validation_mode, ValidationMode::Strict);
    assert_eq!(edge_type.properties[0].name.as_str(), "since");
    assert!(!edge_type.properties[0].required);
    assert!(!edge_type.properties[0].immutable);
    let _ = fs::remove_dir_all(dir);
}
