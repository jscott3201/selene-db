//! Closed-graph type fixtures shared by analyzer and graph tests.

use selene_core::{IStr, LabelSet, PropertyValueType, intern};
use selene_graph::{EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, ValidationMode};

fn istr(value: &str) -> IStr {
    intern(value).expect("fixture strings fit interner")
}

fn property(name: &str, value_type: PropertyValueType, required: bool) -> PropertyTypeDef {
    PropertyTypeDef {
        name: istr(name),
        value_type,
        required,
        default: None,
        immutable: false,
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
            source_node_type: 0,
            target_node_type: 2,
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
