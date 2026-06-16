use selene_core::db_string;

use super::*;
use crate::{ProcedureMutability, ProcedureTier};

fn name(segments: &[&str]) -> Vec<DbString> {
    segments
        .iter()
        .map(|segment| db_string(segment).expect("string fits DB string cap"))
        .collect()
}

#[test]
fn builtin_tiers_and_mutability_match_pack() {
    let registry = BuiltinProcedureRegistry::new();
    // Read-only graph-tier built-ins.
    for builtin in [
        &["selene", "health"][..],
        &["selene", "feature_status"][..],
        &["selene", "verify"][..],
        &["selene", "compaction_stats"][..],
        &["selene", "vector_search_nodes"][..],
        &["selene", "vector_search_nodes_batch"][..],
        &["selene", "vector_score_nodes"][..],
        &["selene", "vector_score_nodes_batch"][..],
        &["selene", "vector_score_neighbors"][..],
        &["selene", "vector_score_neighbors_batch"][..],
        &["selene", "vector_score_candidate_state"][..],
        &["selene", "vector_score_candidate_state_nodes"][..],
        &["selene", "vector_score_candidate_state_expanded"][..],
        &["selene", "vector_score_candidate_state_expanded_batch"][..],
        &["selene", "vector_candidate_states"][..],
        &["selene", "vector_score_expanded_candidates"][..],
        &["selene", "vector_score_expanded_candidates_batch"][..],
        &["selene", "vector_search_nodes_ann"][..],
        &["selene", "vector_search_nodes_ann_batch"][..],
        &["selene", "vector_search_expanded_candidates_ann"][..],
        &["selene", "vector_search_candidate_state_expanded_ann"][..],
        &["selene", "vector_search_expanded_candidates_ann_batch"][..],
        &["selene", "vector_index_stats"][..],
        &["selene", "text_index_stats"][..],
        &["selene", "text_search_nodes"][..],
        &["selene", "text_score_nodes"][..],
        &["selene", "text_score_nodes_batch"][..],
        &["selene", "text_score_candidate_state"][..],
        &["selene", "text_score_candidate_state_nodes"][..],
        &["selene", "text_score_candidate_state_expanded_batch"][..],
        &["selene", "reciprocal_rank_fusion"][..],
    ] {
        let metadata = registry.lookup(&name(builtin)).expect("resolves");
        assert_eq!(metadata.tier, ProcedureTier::Graph, "{builtin:?}");
        assert_eq!(
            metadata.mutability,
            ProcedureMutability::Read,
            "{builtin:?}"
        );
    }
    // Mutation-tier schema-write built-ins.
    for builtin in [
        &["selene", "create_index"][..],
        &["selene", "drop_index"][..],
        &["selene", "create_vector_index"][..],
        &["selene", "drop_vector_index"][..],
    ] {
        let metadata = registry.lookup(&name(builtin)).expect("resolves");
        assert_eq!(metadata.tier, ProcedureTier::Mutation, "{builtin:?}");
        assert_eq!(
            metadata.mutability,
            ProcedureMutability::SchemaWrite,
            "{builtin:?}"
        );
    }
    let metadata = registry
        .lookup(&name(&["selene", "rebuild_vector_indexes"]))
        .expect("rebuild_vector_indexes resolves");
    assert_eq!(metadata.tier, ProcedureTier::Maintenance);
    assert_eq!(metadata.mutability, ProcedureMutability::MaintenanceWrite);
    let metadata = registry
        .lookup(&name(&["selene", "rebuild_recommended_vector_indexes"]))
        .expect("rebuild_recommended_vector_indexes resolves");
    assert_eq!(metadata.tier, ProcedureTier::Maintenance);
    assert_eq!(metadata.mutability, ProcedureMutability::MaintenanceWrite);
    let metadata = registry
        .lookup(&name(&["selene", "compact"]))
        .expect("compact resolves");
    assert_eq!(metadata.tier, ProcedureTier::Maintenance);
    assert_eq!(metadata.mutability, ProcedureMutability::MaintenanceWrite);
}

#[test]
fn verify_signature_has_optional_deep_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "verify"]))
        .expect("verify resolves");
    // Single `deep` BOOLEAN parameter carrying an executable default, so the
    // arity accepts 0 or 1 args (matching the pack's `with_default`).
    assert_eq!(metadata.signature.parameters.len(), 1);
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 1);
    let deep = &metadata.signature.parameters[0];
    assert_eq!(deep.default_doc, Some("false"));
    assert!(deep.default.is_some());
}

#[test]
fn create_index_signature_is_exact_three_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "create_index"]))
        .expect("create_index resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 3);
    assert_eq!(arity.maximum, 3);
    assert!(metadata.output_schema.columns.is_empty());
}

#[test]
fn vector_search_signature_has_optional_metric_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_search_nodes"]))
        .expect("vector_search_nodes resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 5);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 5);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, crate::GqlType::Vector);
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
fn vector_search_ann_signature_has_metric_and_ef_search_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_search_nodes_ann"]))
        .expect("vector_search_nodes_ann resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 8);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 8);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, crate::GqlType::Vector);
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, crate::GqlType::Integer);
    assert_eq!(parameters[4].name.as_str(), "metric");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert!(parameters[4].nullable);
    assert_eq!(
        parameters[4].default_doc,
        Some("NULL (matching index metric, otherwise squared_euclidean)")
    );
    assert_eq!(
        parameters[4].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(parameters[5].name.as_str(), "ef_search");
    assert_eq!(parameters[5].ty, crate::GqlType::Integer);
    assert!(parameters[5].nullable);
    assert_eq!(
        parameters[5].default_doc,
        Some("NULL (HNSW 64, IVF 2, TurboQuant 512)")
    );
    assert_eq!(
        parameters[5].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(parameters[6].name.as_str(), "filter_property");
    assert_eq!(parameters[6].ty, crate::GqlType::String);
    assert!(parameters[6].nullable);
    assert_eq!(parameters[6].default_doc, Some("NULL (no property filter)"));
    assert_eq!(
        parameters[6].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(parameters[7].name.as_str(), "filter_values");
    assert_eq!(
        parameters[7].ty,
        crate::GqlType::List(Box::new(crate::GqlType::AnyProperty))
    );
    assert!(parameters[7].nullable);
    assert_eq!(parameters[7].default_doc, Some("NULL (no property filter)"));
    assert_eq!(
        parameters[7].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
}

#[test]
fn vector_search_batch_signature_has_list_vector_query_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_search_nodes_batch"]))
        .expect("vector_search_nodes_batch resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 5);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 5);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "queries");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Vector))
    );
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, crate::GqlType::Integer);
    assert_eq!(parameters[4].name.as_str(), "metric");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert_eq!(parameters[4].default_doc, Some("squared_euclidean"));
    assert!(parameters[4].default.is_some());

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
fn vector_score_nodes_signature_has_list_node_candidates_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_score_nodes"]))
        .expect("vector_score_nodes resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 5);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 5);
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, crate::GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "nodes");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::NodeRef))
    );
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
fn vector_search_ann_batch_signature_has_list_vector_query_arg() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_search_nodes_ann_batch"]))
        .expect("vector_search_nodes_ann_batch resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 6);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 6);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "queries");
    assert_eq!(
        parameters[2].ty,
        crate::GqlType::List(Box::new(crate::GqlType::Vector))
    );
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, crate::GqlType::Integer);
    assert_eq!(parameters[4].name.as_str(), "metric");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert!(parameters[4].nullable);
    assert_eq!(
        parameters[4].default_doc,
        Some("NULL (matching index metric, otherwise squared_euclidean)")
    );
    assert_eq!(
        parameters[4].default,
        Some(crate::ProcedureDefaultValue::Null)
    );
    assert_eq!(parameters[5].name.as_str(), "ef_search");
    assert_eq!(parameters[5].ty, crate::GqlType::Integer);
    assert!(parameters[5].nullable);
    assert_eq!(
        parameters[5].default_doc,
        Some("NULL (HNSW 64, IVF 2, TurboQuant 512)")
    );
    assert_eq!(
        parameters[5].default,
        Some(crate::ProcedureDefaultValue::Null)
    );

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
fn vector_index_stats_signature_is_zero_arg_read() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "vector_index_stats"]))
        .expect("vector_index_stats resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 0);
    assert_eq!(metadata.tier, ProcedureTier::Graph);
    assert_eq!(metadata.mutability, ProcedureMutability::Read);
    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 43);
    assert_eq!(columns[0].name.as_str(), "name");
    assert_eq!(columns[0].ty, crate::GqlType::String);
    assert_eq!(columns[4].name.as_str(), "dimension");
    assert_eq!(columns[4].ty, crate::GqlType::Uint64);
    assert_eq!(columns[14].name.as_str(), "hnsw_level_zero_link_count");
    assert_eq!(columns[14].ty, crate::GqlType::Uint64);
    assert_eq!(columns[19].name.as_str(), "ivf_index_bytes");
    assert_eq!(columns[19].ty, crate::GqlType::Uint64);
    assert_eq!(columns[26].name.as_str(), "ivf_non_empty_list_count");
    assert_eq!(columns[26].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[28].name.as_str(),
        "ivf_average_list_len_basis_points"
    );
    assert_eq!(columns[28].ty, crate::GqlType::Uint64);
    assert_eq!(columns[30].name.as_str(), "ivf_pending_retrain_entries");
    assert_eq!(columns[30].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[31].name.as_str(),
        "ivf_pending_retrain_basis_points"
    );
    assert_eq!(columns[31].ty, crate::GqlType::Uint64);
    assert_eq!(columns[32].name.as_str(), "ivf_rebuild_recommended");
    assert_eq!(columns[32].ty, crate::GqlType::Boolean);
    assert_eq!(columns[34].name.as_str(), "estimated_reachable_bytes");
    assert_eq!(columns[34].ty, crate::GqlType::Uint64);
    assert_eq!(columns[35].name.as_str(), "turbo_quant_index_bytes");
    assert_eq!(columns[35].ty, crate::GqlType::Uint64);
    assert_eq!(columns[42].name.as_str(), "turbo_quant_calibration_bytes");
    assert_eq!(columns[42].ty, crate::GqlType::Uint64);
}

#[test]
fn rebuild_vector_indexes_signature_is_zero_arg_maintenance() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "rebuild_vector_indexes"]))
        .expect("rebuild_vector_indexes resolves");
    assert_rebuild_vector_indexes_metadata(&metadata);
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 0);
}

#[test]
fn rebuild_recommended_vector_indexes_signature_accepts_optional_cap() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "rebuild_recommended_vector_indexes"]))
        .expect("rebuild_recommended_vector_indexes resolves");
    assert_rebuild_vector_indexes_metadata(&metadata);
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 1);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0].name.as_str(), "max_indexes");
    assert_eq!(parameters[0].ty, crate::GqlType::Integer);
    assert!(parameters[0].nullable);
    assert_eq!(parameters[0].default_doc, Some("NULL"));
    assert!(parameters[0].default.is_some());
}

fn assert_rebuild_vector_indexes_metadata(metadata: &crate::ProcedureMetadata) {
    assert_eq!(metadata.tier, ProcedureTier::Maintenance);
    assert_eq!(metadata.mutability, ProcedureMutability::MaintenanceWrite);
    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 71);
    assert_eq!(columns[0].name.as_str(), "name");
    assert_eq!(columns[0].ty, crate::GqlType::String);
    assert_eq!(columns[19].name.as_str(), "before_hnsw_deleted_entries");
    assert_eq!(columns[19].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[23].name.as_str(),
        "before_hnsw_level_zero_link_count"
    );
    assert_eq!(columns[23].ty, crate::GqlType::Uint64);
    assert_eq!(columns[33].name.as_str(), "before_ivf_index_bytes");
    assert_eq!(columns[33].ty, crate::GqlType::Uint64);
    assert_eq!(columns[47].name.as_str(), "before_ivf_non_empty_list_count");
    assert_eq!(columns[47].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[51].name.as_str(),
        "before_ivf_average_list_len_basis_points"
    );
    assert_eq!(columns[51].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[55].name.as_str(),
        "before_ivf_pending_retrain_entries"
    );
    assert_eq!(columns[55].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[56].name.as_str(),
        "after_ivf_pending_retrain_entries"
    );
    assert_eq!(columns[56].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[57].name.as_str(),
        "before_ivf_pending_retrain_basis_points"
    );
    assert_eq!(columns[57].ty, crate::GqlType::Uint64);
    assert_eq!(
        columns[58].name.as_str(),
        "after_ivf_pending_retrain_basis_points"
    );
    assert_eq!(columns[58].ty, crate::GqlType::Uint64);
    assert_eq!(columns[59].name.as_str(), "before_ivf_rebuild_recommended");
    assert_eq!(columns[59].ty, crate::GqlType::Boolean);
    assert_eq!(columns[60].name.as_str(), "after_ivf_rebuild_recommended");
    assert_eq!(columns[60].ty, crate::GqlType::Boolean);
    assert_eq!(columns[70].name.as_str(), "reclaimed_reachable_bytes");
    assert_eq!(columns[70].ty, crate::GqlType::Uint64);
}

#[test]
fn create_vector_index_signature_has_optional_kind_and_name_args() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["selene", "create_vector_index"]))
        .expect("create_vector_index resolves");
    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 3);
    assert_eq!(arity.maximum, 9);

    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 9);
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, crate::GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, crate::GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "dimension");
    assert_eq!(parameters[2].ty, crate::GqlType::Integer);
    assert_eq!(parameters[3].name.as_str(), "kind");
    assert_eq!(parameters[3].ty, crate::GqlType::String);
    assert_eq!(parameters[3].default_doc, Some("ivf_cosine"));
    assert!(parameters[3].default.is_some());
    assert_eq!(parameters[4].name.as_str(), "name");
    assert_eq!(parameters[4].ty, crate::GqlType::String);
    assert!(parameters[4].nullable);
    assert_eq!(parameters[4].default_doc, Some("NULL"));
    assert!(parameters[4].default.is_some());
    assert_eq!(parameters[5].name.as_str(), "metric");
    assert_eq!(parameters[5].ty, crate::GqlType::String);
    assert!(parameters[5].nullable);
    assert_eq!(parameters[5].default_doc, Some("NULL"));
    assert!(parameters[5].default.is_some());
    assert_eq!(parameters[6].name.as_str(), "hnsw_max_neighbors");
    assert_eq!(parameters[6].ty, crate::GqlType::Integer);
    assert!(parameters[6].nullable);
    assert_eq!(parameters[6].default_doc, Some("NULL"));
    assert!(parameters[6].default.is_some());
    assert_eq!(parameters[7].name.as_str(), "hnsw_ef_construction");
    assert_eq!(parameters[7].ty, crate::GqlType::Integer);
    assert!(parameters[7].nullable);
    assert_eq!(parameters[7].default_doc, Some("NULL"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "ivf_target_centroids");
    assert_eq!(parameters[8].ty, crate::GqlType::Integer);
    assert!(parameters[8].nullable);
    assert_eq!(parameters[8].default_doc, Some("NULL"));
    assert!(parameters[8].default.is_some());
    assert!(metadata.output_schema.columns.is_empty());
}

#[test]
fn registry_version_is_frozen_zero() {
    assert_eq!(BuiltinProcedureRegistry::new().registry_version(), 0);
}

#[test]
fn every_algo_procedure_resolves_by_name() {
    let registry = BuiltinProcedureRegistry::new();
    for spec in &ALGO_SPECS {
        let key = name(spec.name);
        assert!(
            registry.lookup(&key).is_some(),
            "procedure {:?} must resolve",
            spec.name
        );
    }
}

#[test]
fn algo_procedures_are_graph_tier_read() {
    let registry = BuiltinProcedureRegistry::new();
    for spec in &ALGO_SPECS {
        let metadata = registry.lookup(&name(spec.name)).expect("resolves");
        assert_eq!(metadata.tier, ProcedureTier::Graph, "{:?}", spec.name);
        assert_eq!(
            metadata.mutability,
            ProcedureMutability::Read,
            "{:?}",
            spec.name
        );
    }
}

#[test]
fn handles_are_unique_and_one_based() {
    let registry = BuiltinProcedureRegistry::new();
    let mut handles: Vec<u64> = registry
        .iter_handles()
        .map(|(_, metadata)| metadata.handle.raw())
        .collect();
    handles.sort_unstable();
    assert_eq!(handles, (1..=67).collect::<Vec<_>>());
}

#[test]
fn unknown_name_returns_none() {
    let registry = BuiltinProcedureRegistry::new();
    assert!(registry.lookup(&name(&["algo", "nonexistent"])).is_none());
}

#[test]
fn forget_graph_is_idempotent() {
    let registry = BuiltinProcedureRegistry::new();
    // No catalog has been created yet for this id.
    assert!(!registry.forget_graph(GraphId::new(7)));
}
