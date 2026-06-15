//! Metadata coverage for BM25 candidate-state scoring procedures.

use selene_core::DbString;
use selene_gql::{BuiltinProcedureRegistry, GqlType, ProcedureRegistry};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn full_registry() -> BuiltinProcedureRegistry {
    BuiltinProcedureRegistry::new()
}

#[test]
fn text_score_candidate_state_metadata_has_state_name() {
    let registry = full_registry();
    let name = [db_string("selene"), db_string("text_score_candidate_state")];
    let metadata = registry
        .lookup(&name)
        .expect("text_score_candidate_state resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 5);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "state_name");
    assert_eq!(parameters[3].ty, GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, GqlType::Integer);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "score");
    assert_eq!(columns[1].ty, GqlType::Float64);
}

#[test]
fn text_score_candidate_state_nodes_metadata_has_composition_args() {
    let registry = full_registry();
    let name = [
        db_string("selene"),
        db_string("text_score_candidate_state_nodes"),
    ];
    let metadata = registry
        .lookup(&name)
        .expect("text_score_candidate_state_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 6);
    assert_eq!(arity.maximum, 7);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "state_name");
    assert_eq!(parameters[3].ty, GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "nodes");
    assert_eq!(parameters[4].ty, GqlType::List(Box::new(GqlType::NodeRef)));
    assert_eq!(parameters[5].name.as_str(), "k");
    assert_eq!(parameters[5].ty, GqlType::Integer);
    assert_eq!(parameters[6].name.as_str(), "operation");
    assert_eq!(parameters[6].ty, GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("intersection"));
    assert!(parameters[6].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "score");
    assert_eq!(columns[1].ty, GqlType::Float64);
}

#[test]
fn text_score_candidate_state_expanded_batch_metadata_has_state_roots() {
    let registry = full_registry();
    let name = [
        db_string("selene"),
        db_string("text_score_candidate_state_expanded_batch"),
    ];
    let metadata = registry
        .lookup(&name)
        .expect("text_score_candidate_state_expanded_batch resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 7);
    assert_eq!(arity.maximum, 9);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "queries");
    assert_eq!(parameters[2].ty, GqlType::List(Box::new(GqlType::String)));
    assert_eq!(parameters[3].name.as_str(), "state_name");
    assert_eq!(parameters[3].ty, GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "roots");
    assert_eq!(
        parameters[4].ty,
        GqlType::List(Box::new(GqlType::List(Box::new(GqlType::NodeRef))))
    );
    assert_eq!(parameters[5].name.as_str(), "edge_label");
    assert_eq!(parameters[5].ty, GqlType::String);
    assert_eq!(parameters[6].name.as_str(), "k");
    assert_eq!(parameters[6].ty, GqlType::Integer);
    assert_eq!(parameters[7].name.as_str(), "operation");
    assert_eq!(parameters[7].ty, GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("intersection"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "direction");
    assert_eq!(parameters[8].ty, GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("outgoing"));
    assert!(parameters[8].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "score");
    assert_eq!(columns[2].ty, GqlType::Float64);
}
