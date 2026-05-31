//! Closed-graph type fixtures shared by analyzer and graph tests.

use selene_core::{IStr, LabelSet, PropertyValueType, intern};
use selene_graph::{
    EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, ValidationMode,
};

fn istr(value: &str) -> IStr {
    intern(value).expect("fixture strings fit interner")
}

fn property(name: &str, value_type: PropertyValueType, required: bool) -> PropertyTypeDef {
    PropertyTypeDef {
        name: istr(name),
        value_type,
        list_element_type: None,
        required,
        default: None,
        immutable: false,
        record_field_types: None,
    }
}

fn labels(values: &[&str]) -> LabelSet {
    values.iter().map(|value| istr(value)).collect()
}

/// Graph type with `Person`, `Person+Active`, `Company`, and `WORKS_AT`.
#[must_use]
pub fn person_company_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("fixture.person_company"),
        node_types: vec![
            NodeTypeDef {
                name: istr("Person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: vec![
                    property("name", PropertyValueType::String, true),
                    property("nickname", PropertyValueType::String, false),
                ],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("ActivePerson"),
                key_labels: labels(&["Person", "Active"]),
                properties: vec![
                    property("name", PropertyValueType::String, true),
                    property("nickname", PropertyValueType::String, false),
                ],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("Company"),
                key_labels: LabelSet::single(istr("Company")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: istr("WorksAt"),
            label: istr("WORKS_AT"),
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
        name: istr("fixture.person_only"),
        node_types: vec![NodeTypeDef {
            name: istr("Person"),
            key_labels: LabelSet::single(istr("Person")),
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
        name: istr("fixture.mentions_one_of"),
        node_types: vec![
            NodeTypeDef {
                name: istr("Document"),
                key_labels: LabelSet::single(istr("Document")),
                properties: vec![property("title", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("Comment"),
                key_labels: LabelSet::single(istr("Comment")),
                properties: vec![property("body", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("Topic"),
                key_labels: LabelSet::single(istr("Topic")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: istr("Mentions"),
            label: istr("MENTIONS"),
            source_node_type: EdgeEndpointDef::one_of([0, 1]),
            target_node_type: EdgeEndpointDef::NodeType(2),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
    .validate()
    .expect("fixture graph type is valid")
}
