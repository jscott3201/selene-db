//! Analyzer write-set and statement-category tests.

use selene_core::{IStr, intern};
use selene_gql::{
    AnalysisError, AnalyzedStatement, DeleteMode, ElementKind, EmptyProcedureRegistry, GqlType,
    LabelExpr, MutationWriteSet, ProcedureMutability, ProcedureOutputColumn, ProcedureRegistry,
    StatementCategory, WriteKind, analyze, parse,
};
use selene_testing::MockProcedureRegistry;

fn analyze_one(source: &str) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry)
}

fn analyze_with(
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry)
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

fn target_name(analyzed: &AnalyzedStatement, kind: &WriteKind) -> &'static str {
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
    analyzed
        .scopes
        .declaration(target)
        .expect("target binding exists")
        .name()
        .as_str()
}

fn registry_with_mutability(mutability: ProcedureMutability) -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure_mutability(
        vec![istr("pkg"), istr("proc")],
        Vec::new(),
        vec![ProcedureOutputColumn {
            name: istr("result"),
            ty: GqlType::String,
        }],
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
    let WriteKind::SetProperty { element, key, .. } = entries[0].kind else {
        panic!("expected SetProperty");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(key.as_str(), "age");
    assert_eq!(target_name(&analyzed, &entries[0].kind), "n");
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
        .map(|entry| match entry.kind {
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
    let WriteKind::SetLabel { element, label, .. } = entries[0].kind else {
        panic!("expected SetLabel");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(label.as_str(), "Tagged");
}

#[test]
fn remove_property_writeset_records_target_and_key() {
    let analyzed = analyze_one("MATCH (n) REMOVE n.age").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::RemoveProperty { element, key, .. } = entries[0].kind else {
        panic!("expected RemoveProperty");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(key.as_str(), "age");
}

#[test]
fn remove_label_writeset_records_label() {
    let analyzed = analyze_one("MATCH (n) REMOVE n :Tagged").expect("analyzes");
    let entries = &write_set(&analyzed).entries;
    let WriteKind::RemoveLabel { element, label, .. } = entries[0].kind else {
        panic!("expected RemoveLabel");
    };
    assert_eq!(element, ElementKind::Node);
    assert_eq!(label.as_str(), "Tagged");
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
    let analyzed = analyze_one("CREATE GRAPH demo").expect("analyzes");
    assert_eq!(analyzed.category, StatementCategory::CatalogModifying);
    assert!(analyzed.write_set.is_none());
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
            ProcedureMutability::GraphWrite,
            StatementCategory::DataModifying,
        ),
        (
            ProcedureMutability::SchemaWrite,
            StatementCategory::CatalogModifying,
        ),
        (
            ProcedureMutability::Admin,
            StatementCategory::CatalogModifying,
        ),
    ] {
        let registry = registry_with_mutability(mutability);
        let analyzed = analyze_with("CALL pkg.proc() YIELD result", &registry).expect("analyzes");
        assert_eq!(analyzed.category, category);
        assert!(analyzed.write_set.is_none());
    }
}

#[test]
fn query_pipeline_calling_graph_write_procedure_errors() {
    let registry = registry_with_mutability(ProcedureMutability::GraphWrite);
    let err = analyze_with(
        "MATCH (n) CALL pkg.proc() YIELD result RETURN result",
        &registry,
    )
    .expect_err("mutating CALL in read pipeline errors");
    assert!(matches!(
        err,
        AnalysisError::MutatingProcedureInReadPipeline {
            mutability: ProcedureMutability::GraphWrite,
            ..
        }
    ));
    assert_eq!(err.gqlstatus().as_str(), "25G02");
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
fn query_pipeline_calling_admin_procedure_errors() {
    let registry = registry_with_mutability(ProcedureMutability::Admin);
    let err = analyze_with(
        "MATCH (n) CALL pkg.proc() YIELD result RETURN result",
        &registry,
    )
    .expect_err("mutating CALL in read pipeline errors");
    assert!(matches!(
        err,
        AnalysisError::MutatingProcedureInReadPipeline {
            mutability: ProcedureMutability::Admin,
            ..
        }
    ));
    assert_eq!(err.gqlstatus().as_str(), "25G02");
}
