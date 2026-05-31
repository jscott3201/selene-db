use std::sync::Arc;

use selene_core::{GraphId, LabelSet, PropertyValueType, intern};
use selene_persist::RecoveryProvider;

use super::*;
use crate::core_provider::sections::{decode_graph_types, encode_graph_types};
use crate::graph_types::{
    GraphTypeDef, NodeTypeDef, PropertyElementType, PropertyTypeDef, ValidationMode,
};
use crate::{GraphError, SeleneGraph, SharedGraph};

#[test]
fn gtyp_v3_preserves_type_model_fields() {
    let graph_type = GraphTypeDef {
        name: intern("core.gtyp.v3").unwrap(),
        node_types: vec![NodeTypeDef {
            name: intern("core.gtyp.v3.node").unwrap(),
            key_labels: LabelSet::single(intern("V3Node").unwrap()),
            properties: vec![PropertyTypeDef {
                name: intern("core.gtyp.v3.name").unwrap(),
                value_type: PropertyValueType::List,
                list_element_type: Some(PropertyElementType::List(Box::new(
                    PropertyElementType::Scalar(PropertyValueType::String),
                ))),
                required: false,
                default: None,
                immutable: true,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Warn,
        }],
        edge_types: Vec::new(),
    };
    let graph = SharedGraph::builder(GraphId::new(88))
        .bound_to(graph_type.clone())
        .unwrap()
        .build()
        .unwrap()
        .read()
        .as_ref()
        .clone();

    let rows = decode_graph_types(&encode_graph_types(&graph).unwrap()).unwrap();

    assert_eq!(rows, vec![(0, graph_type)]);
}

#[test]
fn finish_recovery_rejects_gtyp_without_meta() {
    // F5 regression: a snapshot whose section table carries CORE/GTYP rows
    // but no CORE/META row is structurally inconsistent — the type rows have
    // no graph identity to bind to. Recovery must error rather than silently
    // downgrading to an open graph.
    let mut graph = SeleneGraph::new(GraphId::new(20));
    graph.meta.bound_type = Some(Arc::new(GraphTypeDef {
        name: intern("test.gtyp.no.meta").unwrap(),
        node_types: vec![NodeTypeDef {
            name: intern("test.node").unwrap(),
            key_labels: LabelSet::single(intern("Test").unwrap()),
            properties: vec![PropertyTypeDef {
                name: intern("name").unwrap(),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,

                record_field_types: None,
            }],
            validation_mode: crate::ValidationMode::Strict,
        }],
        edge_types: vec![],
    }));
    let gtyp_bytes = encode_graph_types(&graph).unwrap();

    // Apply only GTYP, never META.
    let provider = CoreProvider::new_for_recovery();
    RecoveryProvider::read_section(provider.as_ref(), CORE_GTYP_SUB, &gtyp_bytes).unwrap();
    let err = provider
        .finish_recovery(GraphId::new(20), None)
        .expect_err("recovery must fail when GTYP is non-empty but META is missing");
    assert!(matches!(
        err,
        GraphError::Provider(ProviderError::Inconsistent { reason })
            if reason.contains("GTYP non-empty") && reason.contains("META missing")
    ));
}
