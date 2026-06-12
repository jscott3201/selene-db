use selene_core::{DbString, db_string};

use super::*;
use crate::{ProcedureMutability, ProcedureRegistry, ProcedureTier};

fn name(segments: &[&str]) -> Vec<DbString> {
    segments
        .iter()
        .map(|segment| db_string(segment).expect("string fits DB string cap"))
        .collect()
}

#[test]
fn registers_all_sixty_five_procedures() {
    let registry = BuiltinProcedureRegistry::new();
    let handles: Vec<_> = registry.iter_handles().collect();
    assert_eq!(
        handles.len(),
        65,
        "expected 19 algo procedures + 46 platform built-ins"
    );
}

#[test]
fn pagerank_signature_has_optional_orientation_personalization_and_result_filter() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["algo", "pagerank"]))
        .expect("pagerank resolves");
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 9);

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 9);
    for parameter in &parameters[1..5] {
        assert!(parameter.nullable, "{} should be nullable", parameter.name);
        assert_eq!(parameter.default_doc, Some("NULL (use procedure default)"));
        assert!(parameter.default.is_none());
    }

    let orientation = &parameters[5];
    assert_eq!(orientation.name.as_str(), "orientation");
    assert!(orientation.nullable);
    assert_eq!(orientation.ty, crate::GqlType::String);
    assert_eq!(orientation.default_doc, Some("natural"));
    assert_eq!(
        orientation.default,
        Some(crate::ProcedureDefaultValue::String("natural"))
    );

    let personalization = &parameters[6];
    assert_eq!(personalization.name.as_str(), "personalization");
    assert!(personalization.nullable);
    assert_eq!(personalization.default_doc, Some("NULL (uniform teleport)"));
    assert_eq!(
        personalization.default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(
        personalization.ty,
        crate::GqlType::List(Box::new(crate::GqlType::Record(crate::RecordType::Open)))
    );

    let result_label = &parameters[7];
    assert_eq!(result_label.name.as_str(), "result_label");
    assert!(result_label.nullable);
    assert_eq!(result_label.ty, crate::GqlType::String);
    assert_eq!(
        result_label.default_doc,
        Some("NULL (all projection nodes)")
    );
    assert_eq!(
        result_label.default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    let limit = &parameters[8];
    assert_eq!(limit.name.as_str(), "limit");
    assert!(limit.nullable);
    assert_eq!(limit.ty, crate::GqlType::Integer);
    assert_eq!(limit.default_doc, Some("NULL (all matching nodes)"));
    assert_eq!(limit.default, Some(crate::ProcedureDefaultValue::Null));

    assert_eq!(metadata.output_schema.columns.len(), 2);
    assert_eq!(metadata.output_schema.columns[0].name.as_str(), "node_id");
    assert_eq!(metadata.output_schema.columns[1].name.as_str(), "score");
}

#[test]
fn iter_handles_yields_all_forty_six_platform_builtins() {
    let registry = BuiltinProcedureRegistry::new();
    let names: Vec<Vec<String>> = registry
        .iter_handles()
        .map(|(name, _)| {
            name.iter()
                .map(|segment| segment.as_str().to_owned())
                .collect()
        })
        .collect();
    for expected in [
        ["selene", "health"],
        ["selene", "feature_status"],
        ["selene", "verify"],
        ["selene", "compaction_stats"],
        ["selene", "create_index"],
        ["selene", "drop_index"],
        ["selene", "vector_search_nodes"],
        ["selene", "vector_search_nodes_batch"],
        ["selene", "vector_score_nodes"],
        ["selene", "vector_score_nodes_batch"],
        ["selene", "vector_score_neighbors"],
        ["selene", "vector_score_neighbors_batch"],
        ["selene", "vector_score_candidate_state"],
        ["selene", "vector_score_candidate_state_nodes"],
        ["selene", "vector_score_candidate_state_expanded"],
        ["selene", "vector_score_candidate_state_expanded_batch"],
        ["selene", "vector_candidate_states"],
        ["selene", "vector_score_expanded_candidates"],
        ["selene", "vector_score_expanded_candidates_batch"],
        ["selene", "vector_search_nodes_ann"],
        ["selene", "vector_search_nodes_ann_batch"],
        ["selene", "vector_search_expanded_candidates_ann"],
        ["selene", "vector_search_candidate_state_expanded_ann"],
        ["selene", "vector_search_expanded_candidates_ann_batch"],
        ["selene", "vector_index_stats"],
        ["selene", "text_index_stats"],
        ["selene", "json_contains_nodes"],
        ["selene", "json_path_exists_nodes"],
        ["selene", "json_path_contains_nodes"],
        ["selene", "json_path_value_nodes"],
        ["selene", "json_contains_candidate_nodes"],
        ["selene", "json_path_exists_candidate_nodes"],
        ["selene", "json_path_contains_candidate_nodes"],
        ["selene", "json_path_value_candidate_nodes"],
        ["selene", "rebuild_vector_indexes"],
        ["selene", "rebuild_recommended_vector_indexes"],
        ["selene", "compact"],
        ["selene", "create_vector_index"],
        ["selene", "drop_vector_index"],
        ["selene", "create_text_index"],
        ["selene", "drop_text_index"],
        ["selene", "text_search_nodes"],
        ["selene", "text_score_nodes"],
        ["selene", "text_score_nodes_batch"],
        ["selene", "text_score_candidate_state_expanded_batch"],
        ["selene", "reciprocal_rank_fusion"],
    ] {
        let expected: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            names.contains(&expected),
            "SHOW PROCEDURES must list {expected:?}"
        );
    }
}

#[test]
fn reciprocal_rank_fusion_signature_has_optional_constant_and_weights() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "reciprocal_rank_fusion"]))
        .expect("reciprocal_rank_fusion resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 2);
    assert_eq!(arity.maximum, 4);
    assert_eq!(metadata.tier, ProcedureTier::Graph);
    assert_eq!(metadata.mutability, ProcedureMutability::Read);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 4);
    assert_eq!(parameters[0].name.as_str(), "rankings");
    assert_eq!(
        parameters[0].ty,
        crate::GqlType::List(Box::new(crate::GqlType::List(Box::new(
            crate::GqlType::NodeRef
        ))))
    );
    assert_eq!(parameters[1].name.as_str(), "k");
    assert_eq!(parameters[1].ty, crate::GqlType::Integer);
    assert_eq!(parameters[2].name.as_str(), "rank_constant");
    assert_eq!(parameters[2].ty, crate::GqlType::Float64);
    assert_eq!(parameters[2].default_doc, Some("60"));
    assert_eq!(
        parameters[2].default,
        Some(crate::ProcedureDefaultValue::Integer(60))
    );
    assert_eq!(parameters[3].name.as_str(), "weights");
    assert_eq!(
        parameters[3].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Float))
    );
    assert!(parameters[3].nullable);
    assert_eq!(
        parameters[3].default_doc,
        Some("NULL (all rankings weight 1.0)")
    );
    assert_eq!(
        parameters[3].default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, crate::GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "score");
    assert_eq!(columns[1].ty, crate::GqlType::Float64);
}

#[test]
fn compaction_stats_signature_is_zero_arg_read() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "compaction_stats"]))
        .expect("compaction_stats resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 0);
    assert_eq!(metadata.tier, ProcedureTier::Graph);
    assert_eq!(metadata.mutability, ProcedureMutability::Read);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 12);
    assert_eq!(columns[0].name.as_str(), "allocated_nodes");
    assert_eq!(columns[0].ty, crate::GqlType::Uint64);
    assert_eq!(columns[2].name.as_str(), "reclaimable_nodes");
    assert_eq!(columns[2].ty, crate::GqlType::Uint64);
    assert_eq!(columns[8].name.as_str(), "reclaimable_rows");
    assert_eq!(columns[8].ty, crate::GqlType::Uint64);
    assert_eq!(columns[9].name.as_str(), "reclaimable_row_basis_points");
    assert_eq!(columns[9].ty, crate::GqlType::Uint64);
    assert_eq!(columns[10].name.as_str(), "compaction_recommended");
    assert_eq!(columns[10].ty, crate::GqlType::Boolean);
    assert_eq!(columns[11].name.as_str(), "dense");
    assert_eq!(columns[11].ty, crate::GqlType::Boolean);
}

#[test]
fn compact_signature_is_zero_arg_maintenance() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "compact"]))
        .expect("compact resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 0);
    assert_eq!(metadata.tier, ProcedureTier::Maintenance);
    assert_eq!(metadata.mutability, ProcedureMutability::MaintenanceWrite);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 26);
    assert_eq!(columns[0].name.as_str(), "before_allocated_nodes");
    assert_eq!(columns[0].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[9].name.as_str(),
        "before_reclaimable_row_basis_points"
    );
    assert_eq!(columns[9].ty, crate::GqlType::Uint64);
    assert_eq!(columns[10].name.as_str(), "before_compaction_recommended");
    assert_eq!(columns[10].ty, crate::GqlType::Boolean);
    assert_eq!(columns[12].name.as_str(), "reclaimed_nodes");
    assert_eq!(columns[12].ty, crate::GqlType::Uint64);
    assert_eq!(columns[13].name.as_str(), "reclaimed_edges");
    assert_eq!(columns[13].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[23].name.as_str(),
        "after_reclaimable_row_basis_points"
    );
    assert_eq!(columns[23].ty, crate::GqlType::Uint64);
    assert_eq!(columns[24].name.as_str(), "after_compaction_recommended");
    assert_eq!(columns[24].ty, crate::GqlType::Boolean);
    assert_eq!(columns[25].name.as_str(), "after_dense");
    assert_eq!(columns[25].ty, crate::GqlType::Boolean);
}

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
