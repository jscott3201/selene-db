use super::*;

#[test]
fn vector_search_candidate_state_expanded_ann_signature_exposes_state_and_ann_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&[
            "selene",
            "vector_search_candidate_state_expanded_ann",
        ]))
        .expect("vector_search_candidate_state_expanded_ann resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 7);
    assert_eq!(arity.maximum, 11);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 11);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, crate::GqlType::Vector);
    assert_eq!(parameters[3].name.as_str(), "state_name");
    assert_eq!(parameters[3].ty, crate::GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "root_k");
    assert_eq!(parameters[4].ty, crate::GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "edge_label");
    assert_eq!(parameters[5].ty, crate::GqlType::String);
    assert_eq!(parameters[6].name.as_str(), "k");
    assert_eq!(parameters[6].ty, crate::GqlType::Integer);
    assert_eq!(parameters[7].name.as_str(), "operation");
    assert_eq!(parameters[7].ty, crate::GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("intersection"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "direction");
    assert_eq!(parameters[8].ty, crate::GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("outgoing"));
    assert!(parameters[8].default.is_some());
    assert_eq!(parameters[9].name.as_str(), "metric");
    assert_eq!(parameters[9].ty, crate::GqlType::String);
    assert!(parameters[9].nullable);
    assert_eq!(
        parameters[9].default_doc,
        Some("NULL (matching index metric, otherwise squared_euclidean)")
    );
    assert_eq!(
        parameters[9].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(parameters[10].name.as_str(), "ef_search");
    assert_eq!(parameters[10].ty, crate::GqlType::Integer);
    assert_eq!(
        parameters[10].default_doc,
        Some("NULL (HNSW 64, IVF 2, TurboQuant 512)")
    );
    assert!(parameters[10].nullable);
    assert!(parameters[10].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, crate::GqlType::Float64);
}

#[test]
fn vector_search_expanded_candidates_ann_batch_signature_exposes_root_and_final_k() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&[
            "selene",
            "vector_search_expanded_candidates_ann_batch",
        ]))
        .expect("vector_search_expanded_candidates_ann_batch resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 6);
    assert_eq!(arity.maximum, 9);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 9);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "queries");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Vector))
    );
    assert_eq!(parameters[3].name.as_str(), "root_k");
    assert_eq!(parameters[3].ty, crate::GqlType::Integer);
    assert_eq!(parameters[4].name.as_str(), "edge_label");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert_eq!(parameters[5].name.as_str(), "k");
    assert_eq!(parameters[5].ty, crate::GqlType::Integer);
    assert_eq!(parameters[6].name.as_str(), "direction");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("outgoing"));
    assert!(parameters[6].default.is_some());
    assert_eq!(parameters[7].name.as_str(), "metric");
    assert_eq!(parameters[7].ty, crate::GqlType::String);
    assert!(parameters[7].nullable);
    assert_eq!(
        parameters[7].default_doc,
        Some("NULL (matching index metric, otherwise squared_euclidean)")
    );
    assert_eq!(
        parameters[7].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(parameters[8].name.as_str(), "ef_search");
    assert_eq!(parameters[8].ty, crate::GqlType::Integer);
    assert_eq!(
        parameters[8].default_doc,
        Some("NULL (HNSW 64, IVF 2, TurboQuant 512)")
    );
    assert!(parameters[8].nullable);
    assert!(parameters[8].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, crate::GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "distance");
    assert_eq!(columns[2].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_expanded_candidates_batch_signature_has_nested_roots_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_expanded_candidates_batch"]))
        .expect("vector_score_expanded_candidates_batch resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 7);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 7);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "queries");
    assert_eq!(
        parameters[1].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Vector))
    );
    assert_eq!(parameters[2].name.as_str(), "roots");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::List(Box::new(
            crate::GqlType::NodeRef
        ))))
    );
    assert_eq!(parameters[3].name.as_str(), "edge_label");
    assert_eq!(parameters[3].ty, crate::GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, crate::GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "direction");
    assert_eq!(parameters[5].ty, crate::GqlType::String);
    assert_eq!(parameters[5].default_doc, Some("outgoing"));
    assert!(parameters[5].default.is_some());
    assert_eq!(parameters[6].name.as_str(), "metric");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("squared_euclidean"));
    assert!(parameters[6].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, crate::GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "distance");
    assert_eq!(columns[2].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_expanded_candidates_signature_has_root_expansion_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_expanded_candidates"]))
        .expect("vector_score_expanded_candidates resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 7);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 7);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, crate::GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "roots");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::NodeRef))
    );
    assert_eq!(parameters[3].name.as_str(), "edge_label");
    assert_eq!(parameters[3].ty, crate::GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, crate::GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "direction");
    assert_eq!(parameters[5].ty, crate::GqlType::String);
    assert_eq!(parameters[5].default_doc, Some("outgoing"));
    assert!(parameters[5].default.is_some());
    assert_eq!(parameters[6].name.as_str(), "metric");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("squared_euclidean"));
    assert!(parameters[6].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_candidate_state_signature_has_state_name_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_candidate_state"]))
        .expect("vector_score_candidate_state resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 5);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 5);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, crate::GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "state_name");
    assert_eq!(parameters[2].ty, crate::GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, crate::GqlType::Integer);
    assert_eq!(parameters[4].name.as_str(), "metric");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert_eq!(parameters[4].default_doc, Some("squared_euclidean"));
    assert!(parameters[4].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_candidate_state_expanded_signature_has_state_and_expansion_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_candidate_state_expanded"]))
        .expect("vector_score_candidate_state_expanded resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 6);
    assert_eq!(arity.maximum, 9);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 9);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, crate::GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "state_name");
    assert_eq!(parameters[2].ty, crate::GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "roots");
    assert_eq!(
        parameters[3].ty,
        crate::GqlType::List(Box::new(crate::GqlType::NodeRef))
    );
    assert_eq!(parameters[4].name.as_str(), "edge_label");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert_eq!(parameters[5].name.as_str(), "k");
    assert_eq!(parameters[5].ty, crate::GqlType::Integer);
    assert_eq!(parameters[6].name.as_str(), "operation");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("intersection"));
    assert!(parameters[6].default.is_some());
    assert_eq!(parameters[7].name.as_str(), "direction");
    assert_eq!(parameters[7].ty, crate::GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("outgoing"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "metric");
    assert_eq!(parameters[8].ty, crate::GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("squared_euclidean"));
    assert!(parameters[8].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_candidate_state_expanded_batch_signature_has_state_and_nested_roots_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&[
            "selene",
            "vector_score_candidate_state_expanded_batch",
        ]))
        .expect("vector_score_candidate_state_expanded_batch resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 6);
    assert_eq!(arity.maximum, 9);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 9);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "queries");
    assert_eq!(
        parameters[1].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Vector))
    );
    assert_eq!(parameters[2].name.as_str(), "state_name");
    assert_eq!(parameters[2].ty, crate::GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "roots");
    assert_eq!(
        parameters[3].ty,
        crate::GqlType::List(Box::new(crate::GqlType::List(Box::new(
            crate::GqlType::NodeRef
        ))))
    );
    assert_eq!(parameters[4].name.as_str(), "edge_label");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert_eq!(parameters[5].name.as_str(), "k");
    assert_eq!(parameters[5].ty, crate::GqlType::Integer);
    assert_eq!(parameters[6].name.as_str(), "operation");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("intersection"));
    assert!(parameters[6].default.is_some());
    assert_eq!(parameters[7].name.as_str(), "direction");
    assert_eq!(parameters[7].ty, crate::GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("outgoing"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "metric");
    assert_eq!(parameters[8].ty, crate::GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("squared_euclidean"));
    assert!(parameters[8].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, crate::GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "distance");
    assert_eq!(columns[2].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_neighbors_signature_has_anchor_and_direction_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_neighbors"]))
        .expect("vector_score_neighbors resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 7);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 7);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, crate::GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "anchor");
    assert_eq!(parameters[2].ty, crate::GqlType::NodeRef);
    assert_eq!(parameters[3].name.as_str(), "edge_label");
    assert_eq!(parameters[3].ty, crate::GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, crate::GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "direction");
    assert_eq!(parameters[5].ty, crate::GqlType::String);
    assert_eq!(parameters[5].default_doc, Some("outgoing"));
    assert!(parameters[5].default.is_some());
    assert_eq!(parameters[6].name.as_str(), "metric");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("squared_euclidean"));
    assert!(parameters[6].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, crate::GqlType::Float64);
}

#[test]
fn vector_score_neighbors_batch_signature_has_anchor_list_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_neighbors_batch"]))
        .expect("vector_score_neighbors_batch resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 7);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 7);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "queries");
    assert_eq!(
        parameters[1].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Vector))
    );
    assert_eq!(parameters[2].name.as_str(), "anchors");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::NodeRef))
    );
    assert_eq!(parameters[3].name.as_str(), "edge_label");
    assert_eq!(parameters[3].ty, crate::GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, crate::GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "direction");
    assert_eq!(parameters[5].ty, crate::GqlType::String);
    assert_eq!(parameters[5].default_doc, Some("outgoing"));
    assert!(parameters[5].default.is_some());
    assert_eq!(parameters[6].name.as_str(), "metric");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("squared_euclidean"));
    assert!(parameters[6].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, crate::GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "distance");
    assert_eq!(columns[2].ty, crate::GqlType::Float64);
}
