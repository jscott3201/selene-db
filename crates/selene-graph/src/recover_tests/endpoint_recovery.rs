use std::fs;

use selene_core::{
    Change, EdgeEndpointDef as CoreEdgeEndpointDef, GraphId, LabelSet, SchemaChange, intern,
};

use super::*;
use crate::EdgeEndpointDef;

#[test]
fn recover_closed_wal_only_preserves_any_edge_endpoints() {
    let dir = temp_dir("closed-schema-any-edge-wal-only");
    let graph_id = GraphId::new(22);
    let person = intern("RecoverAnyPerson").unwrap();
    let company = intern("RecoverAnyCompany").unwrap();
    let rel = intern("RECOVER_ANY_REL").unwrap();
    let base = GraphTypeDef {
        name: intern("recover.any.edge.graph").unwrap(),
        node_types: vec![
            crate::NodeTypeDef {
                name: person.clone(),
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            crate::NodeTypeDef {
                name: company.clone(),
                key_labels: LabelSet::single(company),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_edge_type(
                rel.clone(),
                rel,
                EdgeEndpointDef::Any,
                EdgeEndpointDef::Any,
                Vec::new(),
                ValidationMode::Strict,
            )
            .unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(
        graph_type.edge_types[0].source_node_type,
        EdgeEndpointDef::Any
    );
    assert_eq!(
        graph_type.edge_types[0].target_node_type,
        EdgeEndpointDef::Any
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::EdgeTypeAddedV2 { def, .. },
            ..
        }] if def.source_node_type == CoreEdgeEndpointDef::Any
            && def.target_node_type == CoreEdgeEndpointDef::Any
    ));
    let _ = fs::remove_dir_all(dir);
}
