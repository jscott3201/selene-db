//! Closed-graph type fixtures shared by analyzer and graph tests.

use selene_core::{DbString, LabelSet, PropertyValueType};
use selene_graph::{
    EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, ValidationMode,
};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("fixture strings fit DB string cap")
}

fn property(name: &str, value_type: PropertyValueType, required: bool) -> PropertyTypeDef {
    PropertyTypeDef {
        name: db_string(name),
        value_type,
        list_element_type: None,
        required,
        default: None,
        immutable: false,
        unique: false,
        decimal_type: None,
        byte_string_type: None,
        record_field_types: None,
    }
}

fn labels(values: &[&str]) -> LabelSet {
    values.iter().map(|value| db_string(value)).collect()
}

/// Graph type with `Person`, `Person+Active`, `Company`, and `WORKS_AT`.
#[must_use]
pub fn person_company_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("fixture.person_company"),
        node_types: vec![
            NodeTypeDef {
                name: db_string("Person"),
                key_labels: LabelSet::single(db_string("Person")),
                properties: vec![
                    property("name", PropertyValueType::String, true),
                    property("nickname", PropertyValueType::String, false),
                ],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("ActivePerson"),
                key_labels: labels(&["Person", "Active"]),
                properties: vec![
                    property("name", PropertyValueType::String, true),
                    property("nickname", PropertyValueType::String, false),
                ],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("Company"),
                key_labels: LabelSet::single(db_string("Company")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: db_string("WorksAt"),
            label: db_string("WORKS_AT"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(2),
            properties: vec![property("since", PropertyValueType::Int, false)],
            validation_mode: ValidationMode::Strict,
        }],
    }
    .validate()
    .expect("fixture graph type is valid")
}

/// Minimal closed graph type with one required `Person.name` property.
#[must_use]
pub fn person_only_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("fixture.person_only"),
        node_types: vec![NodeTypeDef {
            name: db_string("Person"),
            key_labels: LabelSet::single(db_string("Person")),
            properties: vec![property("name", PropertyValueType::String, true)],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
    .validate()
    .expect("fixture graph type is valid")
}

/// Graph type exercising `EdgeEndpointDef::OneOf`: a `MENTIONS` edge whose
/// source enumerates `{Document, Comment}` and whose target is the single
/// `Topic` node type.
#[must_use]
pub fn mentions_one_of_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("fixture.mentions_one_of"),
        node_types: vec![
            NodeTypeDef {
                name: db_string("Document"),
                key_labels: LabelSet::single(db_string("Document")),
                properties: vec![property("title", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("Comment"),
                key_labels: LabelSet::single(db_string("Comment")),
                properties: vec![property("body", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("Topic"),
                key_labels: LabelSet::single(db_string("Topic")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: db_string("Mentions"),
            label: db_string("MENTIONS"),
            source_node_type: EdgeEndpointDef::one_of([0, 1]),
            target_node_type: EdgeEndpointDef::NodeType(2),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
    .validate()
    .expect("fixture graph type is valid")
}
