//! Analyzer write-set and statement-category tests.

use selene_core::{IStr, intern};
use selene_gql::{
    AnalysisError, AnalyzedStatement, BindingId, DeleteMode, ElementKind, EmptyProcedureRegistry,
    GqlStatus, GqlType, LabelExpr, MutationWriteSet, ProcedureMutability, ProcedureOutputColumn,
    ProcedureRegistry, SourceSpan, StatementCategory, WriteKind, analyze, parse,
};
use selene_testing::MockProcedureRegistry;

fn analyze_one(source: &str) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None)
}

fn analyze_with(
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry, None)
}

fn write_set(analyzed: &AnalyzedStatement) -> &MutationWriteSet {
    analyzed.write_set.as_ref().expect("mutation write-set")
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test strings fit interner")
}

fn label_name(label_expr: &Option<LabelExpr>) -> &str {
    let Some(LabelExpr::Single(label)) = label_expr else {
        panic!("expected single label");
    };
    label.as_str()
}

fn property_names(keys: &[IStr]) -> Vec<&str> {
    keys.iter().map(|key| key.as_str()).collect()
}

fn binding_name(analyzed: &AnalyzedStatement, binding: BindingId) -> String {
    analyzed
        .scopes
        .declaration(binding)
        .expect("binding exists")
        .name()
        .as_str()
        .to_owned()
}

fn target_name(analyzed: &AnalyzedStatement, kind: &WriteKind) -> String {
    let target = match kind {
        WriteKind::SetProperty { target, .. }
        | WriteKind::SetLabel { target, .. }
        | WriteKind::RemoveProperty { target, .. }
        | WriteKind::RemoveLabel { target, .. }
        | WriteKind::DeleteTarget { target, .. } => *target,
        WriteKind::InsertNode { .. } | WriteKind::InsertEdge { .. } => {
            panic!("insert entries do not have target names")
        }
        _ => panic!("unexpected future write kind"),
    };
    binding_name(analyzed, target)
}

fn insert_binding_name(analyzed: &AnalyzedStatement, kind: &WriteKind) -> Option<String> {
    let binding = match kind {
        WriteKind::InsertNode { binding, .. } | WriteKind::InsertEdge { binding, .. } => *binding,
        _ => panic!("expected insert write kind"),
    };
    binding.map(|binding| binding_name(analyzed, binding))
}

fn span_text(source: &str, span: SourceSpan) -> &str {
    let start = span.byte_offset as usize;
    let end = span.end() as usize;
    &source[start..end]
}

fn registry_with_mutability(mutability: ProcedureMutability) -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure_mutability(
        vec![istr("pkg"), istr("proc")],
        Vec::new(),
        vec![ProcedureOutputColumn::new(istr("result"), GqlType::String)],
        mutability,
    )
}

#[test]
fn insert_node_writeset_records_binding_label_and_keys() {
    let analyzed = analyze_one("INSERT (n:Person { name: 'a' })").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 1);
    let WriteKind::InsertNode {
        binding,
        label_expr,
        property_keys,
    } = &entries[0].kind
    else {
        panic!("expected InsertNode");
    };
    assert!(binding.is_some());
    assert_eq!(label_name(label_expr), "Person");
    assert_eq!(property_names(property_keys), ["name"]);
}

#[test]
fn anonymous_insert_node_emits_entry_without_binding() {
    let analyzed = analyze_one("INSERT (:Person)").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 1);
    let WriteKind::InsertNode {
        binding,
        label_expr,
        property_keys,
    } = &entries[0].kind
    else {
        panic!("expected InsertNode");
    };
    assert_eq!(*binding, None);
    assert_eq!(label_name(label_expr), "Person");
    assert!(property_keys.is_empty());
}

#[test]
fn match_then_insert_edge_only_emits_one_entry() {
    let analyzed = analyze_one("MATCH (a), (b) INSERT (a)-[r:KNOWS]->(b)").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 1);
    let WriteKind::InsertEdge {
        binding,
        label_expr,
        property_keys,
    } = &entries[0].kind
    else {
        panic!("expected InsertEdge");
    };
    assert!(binding.is_some());
    assert_eq!(label_name(label_expr), "KNOWS");
    assert!(property_keys.is_empty());
}

#[test]
fn insert_path_without_prior_match_emits_fresh_nodes_and_edge() {
    let analyzed = analyze_one("INSERT (a)-[r:KNOWS]->(b)").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 3);
    assert!(matches!(
        entries[0].kind,
        WriteKind::InsertNode {
            binding: Some(_),
            ..
        }
    ));
    assert!(matches!(
        entries[1].kind,
        WriteKind::InsertEdge {
            binding: Some(_),
            ..
        }
    ));
    assert!(matches!(
        entries[2].kind,
        WriteKind::InsertNode {
            binding: Some(_),
            ..
        }
    ));
}

#[test]
fn insert_path_with_anonymous_edge_emits_edge_without_binding() {
    let analyzed = analyze_one("INSERT (a)-[:K]->(b)").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 3);
    let WriteKind::InsertEdge { binding, .. } = entries[1].kind else {
        panic!("expected middle edge entry");
    };
    assert_eq!(binding, None);
}

#[test]
fn insert_reused_binding_emits_no_duplicate_entry() {
    let source = "INSERT (a) INSERT (a)-[:K]->(b)";
    let analyzed = analyze_one(source).expect("analyzes");
    let entries = &write_set(&analyzed).entries;

    assert_eq!(entries.len(), 3);
    assert_eq!(span_text(source, entries[0].span), "(a)");
    assert_eq!(span_text(source, entries[2].span), "(b)");

    let WriteKind::InsertNode {
        binding: first_a,
        label_expr,
        property_keys,
    } = &entries[0].kind
    else {
        panic!("expected first INSERT node");
    };
    let Some(first_a) = *first_a else {
        panic!("expected first INSERT node binding");
    };
    assert_eq!(binding_name(&analyzed, first_a), "a");
    assert!(label_expr.is_none());
    assert!(property_keys.is_empty());

    let WriteKind::InsertEdge { binding, .. } = entries[1].kind else {
        panic!("expected anonymous INSERT edge");
    };
    assert_eq!(binding, None);

    assert_eq!(
        insert_binding_name(&analyzed, &entries[2].kind).as_deref(),
        Some("b")
    );
    let emitted_a_count = entries
        .iter()
        .filter(|entry| {
            matches!(
                &entry.kind,
                WriteKind::InsertNode {
                    binding: Some(binding),
                    ..
                } if *binding == first_a
            )
        })
        .count();
    assert_eq!(emitted_a_count, 1);
}

#[test]
fn insert_preserves_label_expression_shape() {
    let analyzed = analyze_one("INSERT (n:Person&Customer)").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::InsertNode { label_expr, .. } = &entries[0].kind else {
        panic!("expected InsertNode");
    };
    assert!(matches!(label_expr, Some(LabelExpr::Conjunction(parts)) if parts.len() == 2));
}

#[test]
fn set_property_writeset_records_target_and_key() {
    let analyzed = analyze_one("MATCH (n) SET n.age = 18").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 1);
    let WriteKind::SetProperty {
        element, ref key, ..
    } = entries[0].kind
    else {
        panic!("expected SetProperty");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(key.as_str(), "age");
    assert_eq!(target_name(&analyzed, &entries[0].kind), "n");
}

#[test]
fn analyzer_resolves_set_target_by_binding_id_not_name() {
    let analyzed = analyze_one("MATCH (m), (a) SET m.age = 18, a.age = 21").expect("analyzes");
    let targets = write_set(&analyzed)
        .entries
        .iter()
        .map(|entry| match entry.kind {
            WriteKind::SetProperty { target, .. } => target,
            _ => panic!("expected SetProperty"),
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 2);
    assert_ne!(targets[0], targets[1]);
    assert_eq!(binding_name(&analyzed, targets[0]), "m");
    assert_eq!(binding_name(&analyzed, targets[1]), "a");
}

#[test]
fn set_property_map_writeset_emits_one_entry_per_key() {
    let analyzed = analyze_one("MATCH (n) SET n = { a: 1, b: 2 }").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 2);
    assert!(matches!(entries[0].kind, WriteKind::SetProperty { .. }));
    assert!(matches!(entries[1].kind, WriteKind::SetProperty { .. }));
    let keys = entries
        .iter()
        .map(|entry| match &entry.kind {
            WriteKind::SetProperty { key, .. } => key.as_str(),
            _ => panic!("expected SetProperty"),
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["a", "b"]);
}

#[test]
fn set_label_writeset_records_label() {
    let analyzed = analyze_one("MATCH (n) SET n :Tagged").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::SetLabel {
        element, ref label, ..
    } = entries[0].kind
    else {
        panic!("expected SetLabel");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(label.as_str(), "Tagged");
}

#[test]
fn remove_property_writeset_records_target_and_key() {
    let analyzed = analyze_one("MATCH (n) REMOVE n.age").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::RemoveProperty {
        element, ref key, ..
    } = entries[0].kind
    else {
        panic!("expected RemoveProperty");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(key.as_str(), "age");
}

#[test]
fn analyzer_resolves_remove_target_by_binding_id_not_name() {
    let analyzed = analyze_one("MATCH (m), (a) REMOVE m.age, a.age").expect("analyzes");
    let targets = write_set(&analyzed)
        .entries
        .iter()
        .map(|entry| match entry.kind {
            WriteKind::RemoveProperty { target, .. } => target,
            _ => panic!("expected RemoveProperty"),
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 2);
    assert_ne!(targets[0], targets[1]);
    assert_eq!(binding_name(&analyzed, targets[0]), "m");
    assert_eq!(binding_name(&analyzed, targets[1]), "a");
}

#[test]
fn remove_label_writeset_records_label() {
    let analyzed = analyze_one("MATCH (n) REMOVE n :Tagged").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::RemoveLabel {
        element, ref label, ..
    } = entries[0].kind
    else {
        panic!("expected RemoveLabel");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(label.as_str(), "Tagged");
}

#[test]
fn remove_edge_label_rejects_at_analyzer() {
    let error = analyze_one("MATCH ()-[r:REL]->() REMOVE r :REL").expect_err("edge label removal");

    assert!(matches!(error, AnalysisError::NotImplemented { .. }));
    assert_eq!(error.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
}

#[test]
fn delete_writeset_records_mode() {
    let analyzed = analyze_one("MATCH (n) DETACH DELETE n").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::DeleteTarget { element, mode, .. } = entries[0].kind else {
        panic!("expected DeleteTarget");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(mode, DeleteMode::Detach);
}

#[test]
fn combined_pipeline_writeset_preserves_source_order() {
    let analyzed = analyze_one("MATCH (n) SET n.a = 1 REMOVE n.b DELETE n").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0].kind, WriteKind::SetProperty { .. }));
    assert!(matches!(entries[1].kind, WriteKind::RemoveProperty { .. }));
    assert!(matches!(entries[2].kind, WriteKind::DeleteTarget { .. }));
}

#[test]
fn query_pipeline_classifies_as_readonly() {
    let analyzed = analyze_one("MATCH (n) RETURN n").expect("analyzes");
    assert_eq!(analyzed.category, StatementCategory::ReadOnly);
    assert!(analyzed.write_set.is_none());
}

#[test]
fn mutation_pipeline_classifies_as_data_modifying() {
    let analyzed = analyze_one("MATCH (n) DELETE n").expect("analyzes");
    assert_eq!(analyzed.category, StatementCategory::DataModifying);
    assert!(analyzed.write_set.is_some());
}

#[test]
fn create_graph_classifies_as_catalog_modifying() {
    let error = parse("CREATE GRAPH demo").expect_err("graph management is outside v1.0");
    assert_eq!(error.gqlstatus().as_str(), "42N01");
}

#[test]
fn show_node_types_classifies_as_readonly() {
    let analyzed = analyze_one("SHOW NODE TYPES").expect("analyzes");
    assert_eq!(analyzed.category, StatementCategory::ReadOnly);
    assert!(analyzed.write_set.is_none());
}

#[test]
fn transaction_control_classifies_correctly() {
    for source in ["START TRANSACTION", "COMMIT", "ROLLBACK"] {
        let analyzed = analyze_one(source).expect("analyzes");
        assert_eq!(analyzed.category, StatementCategory::TransactionControl);
        assert!(analyzed.write_set.is_none());
    }
}

#[test]
fn top_level_call_classification_follows_procedure_mutability() {
    for (mutability, category) in [
        (ProcedureMutability::Read, StatementCategory::ReadOnly),
        (
            ProcedureMutability::SchemaWrite,
            StatementCategory::CatalogModifying,
        ),
        (
            ProcedureMutability::MaintenanceWrite,
            StatementCategory::Maintenance,
        ),
    ] {
        let registry = registry_with_mutability(mutability);
        let analyzed = analyze_with("CALL pkg.proc() YIELD result", &registry).expect("analyzes");
        assert_eq!(analyzed.category, category);
        assert!(analyzed.write_set.is_none());
    }
}

#[test]
fn query_pipeline_calling_schema_write_procedure_errors() {
    let registry = registry_with_mutability(ProcedureMutability::SchemaWrite);
    let err = analyze_with(
        "MATCH (n) CALL pkg.proc() YIELD result RETURN result",
        &registry,
    )
    .expect_err("mutating CALL in read pipeline errors");
    assert!(matches!(
        err,
        AnalysisError::MutatingProcedureInReadPipeline {
            mutability: ProcedureMutability::SchemaWrite,
            ..
        }
    ));
    assert_eq!(err.gqlstatus().as_str(), "25G02");
}

#[test]
fn query_pipeline_calling_maintenance_write_procedure_errors() {
    let registry = registry_with_mutability(ProcedureMutability::MaintenanceWrite);
    let err = analyze_with(
        "MATCH (n) CALL pkg.proc() YIELD result RETURN result",
        &registry,
    )
    .expect_err("maintenance CALL in read pipeline errors");
    assert!(matches!(
        err,
        AnalysisError::MutatingProcedureInReadPipeline {
            mutability: ProcedureMutability::MaintenanceWrite,
            ..
        }
    ));
    assert_eq!(err.gqlstatus().as_str(), "25G02");
}
