use selene_core::{LabelSet, db_string};

use crate::{NodeTypeDef, ValidationMode};

#[path = "variant_tests/legacy_catalog.rs"]
mod legacy_catalog;
#[path = "variant_tests/row_replay.rs"]
mod row_replay;
#[path = "variant_tests/schema_replay.rs"]
mod schema_replay;

fn person_closed_graph_type() -> crate::GraphTypeDef {
    let person = db_string("recover.closed.person").unwrap();
    crate::GraphTypeDef {
        name: db_string("recover.closed.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person.clone(),
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn colliding_legacy_endpoint_graph_type() -> crate::GraphTypeDef {
    let legacy_label = db_string("recover.legacy.person").unwrap();
    crate::GraphTypeDef {
        name: db_string("recover.legacy.collision.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: db_string("recover.legacy.person.type").unwrap(),
                key_labels: LabelSet::single(legacy_label.clone()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: legacy_label,
                key_labels: LabelSet::single(db_string("recover.legacy.other").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    }
}
