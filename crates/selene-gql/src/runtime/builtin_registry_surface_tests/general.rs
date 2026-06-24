use super::*;

#[test]
fn registers_all_sixty_eight_procedures() {
    let registry = BuiltinProcedureRegistry::new();
    let handles: Vec<_> = registry.iter_handles().collect();
    assert_eq!(
        handles.len(),
        68,
        "expected 19 algo procedures + 49 platform built-ins"
    );
}

#[test]
fn pagerank_signature_has_optional_orientation_personalization_and_result_filter() {
    let registry = BuiltinProcedureRegistry::new();
    let metadata = registry
        .lookup(&name(&["algo", "pagerank"]))
        .expect("pagerank resolves");
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters.len(), 14);

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 14);
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

    let result_nodes = &parameters[9];
    assert_eq!(result_nodes.name.as_str(), "result_nodes");
    assert!(result_nodes.nullable);
    assert_eq!(
        result_nodes.ty,
        crate::GqlType::List(Box::new(crate::GqlType::NodeRef))
    );
    assert_eq!(result_nodes.default_doc, Some("NULL (all matching nodes)"));
    assert_eq!(
        result_nodes.default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    let edge_filter_label = &parameters[10];
    assert_eq!(edge_filter_label.name.as_str(), "edge_filter_label");
    assert!(edge_filter_label.nullable);
    assert_eq!(edge_filter_label.ty, crate::GqlType::String);
    assert_eq!(edge_filter_label.default_doc, Some("NULL (no edge filter)"));
    assert_eq!(
        edge_filter_label.default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    let edge_filter_property = &parameters[11];
    assert_eq!(edge_filter_property.name.as_str(), "edge_filter_property");
    assert!(edge_filter_property.nullable);
    assert_eq!(edge_filter_property.ty, crate::GqlType::String);
    assert_eq!(
        edge_filter_property.default_doc,
        Some("NULL (no edge filter)")
    );
    assert_eq!(
        edge_filter_property.default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    let edge_filter_values = &parameters[12];
    assert_eq!(edge_filter_values.name.as_str(), "edge_filter_values");
    assert!(edge_filter_values.nullable);
    assert_eq!(
        edge_filter_values.ty,
        crate::GqlType::List(Box::new(crate::GqlType::AnyProperty))
    );
    assert_eq!(
        edge_filter_values.default_doc,
        Some("NULL (no edge filter)")
    );
    assert_eq!(
        edge_filter_values.default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    let edge_filter_endpoint = &parameters[13];
    assert_eq!(edge_filter_endpoint.name.as_str(), "edge_filter_endpoint");
    assert!(edge_filter_endpoint.nullable);
    assert_eq!(edge_filter_endpoint.ty, crate::GqlType::String);
    assert_eq!(
        edge_filter_endpoint.default_doc,
        Some("NULL (no edge filter)")
    );
    assert_eq!(
        edge_filter_endpoint.default,
        Some(crate::ProcedureDefaultValue::Null)
    );

    assert_eq!(metadata.output_schema.columns.len(), 2);
    assert_eq!(metadata.output_schema.columns[0].name.as_str(), "node_id");
    assert_eq!(metadata.output_schema.columns[1].name.as_str(), "score");
}

#[test]
fn iter_handles_yields_all_forty_nine_platform_builtins() {
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
        ["selene", "reachable_nodes"],
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
        ["selene", "text_score_candidate_state"],
        ["selene", "text_score_candidate_state_nodes"],
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
