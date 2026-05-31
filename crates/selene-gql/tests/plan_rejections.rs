//! PLAN-17 / PLAN-22 / PLAN-23: planner `NotImplemented` /
//! `WriteSetPatternMismatch` / `ProcedureMetadataMismatch` rejection-site
//! coverage.
//!
//! Most of the planner's defensive rejection sites are unreachable from real
//! GQL — the flagger (parse-time) or analyzer rejects the construct first
//! (e.g. variable-scope CALL flags GP03 at parse, `IN TRANSACTIONS` is rejected
//! at analyze, empty / non-alternating / edge-without-target patterns are
//! syntax errors). The `feature` tag on `PlannerError::NotImplemented` is
//! documented "asserted by tests", but only a couple of tags were exact-match
//! pinned. To exercise the planner branches themselves, these tests parse a
//! valid base statement, analyze it, then surgically mutate the analyzed AST
//! into the rejected shape and drive the planner directly — making each
//! defensive guard live and pinning its exact tag.

use selene_core::intern;
use selene_gql::{
    AnalyzedStatement, AnalyzedStatementKind, EmptyProcedureRegistry, MatchClause, MatchMode,
    MutationStatement, PatternElement, PipelineStatement, PlannerError, WriteKind, YieldColumn,
    YieldItem, analyze, parse, plan,
};

fn analyzed(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("base source parses");
    analyze(statement, &EmptyProcedureRegistry, None).expect("base source analyzes")
}

fn plan_err(analyzed: &AnalyzedStatement) -> PlannerError {
    plan(analyzed, &EmptyProcedureRegistry).expect_err("mutated AST must fail planning")
}

/// Apply `mutate` to the leading MATCH clause's pattern elements of a Query.
fn mutate_match_pattern(
    analyzed: &mut AnalyzedStatement,
    mutate: impl FnOnce(&mut Vec<PatternElement>),
) {
    let AnalyzedStatementKind::Query(query) = &mut analyzed.statement else {
        panic!("expected query statement");
    };
    for statement in &mut query.statements {
        if let PipelineStatement::Match(clause) = statement {
            mutate(&mut clause.patterns[0].elements);
            return;
        }
    }
    panic!("no MATCH clause to mutate");
}

/// Apply `mutate` to the leading MATCH clause of a Query.
fn mutate_match_clause(analyzed: &mut AnalyzedStatement, mutate: impl FnOnce(&mut MatchClause)) {
    let AnalyzedStatementKind::Query(query) = &mut analyzed.statement else {
        panic!("expected query statement");
    };
    for statement in &mut query.statements {
        if let PipelineStatement::Match(clause) = statement {
            mutate(clause);
            return;
        }
    }
    panic!("no MATCH clause to mutate");
}

/// Apply `mutate` to the leading inline `CALL { ... }` subquery of a Query.
fn mutate_call_subquery(
    analyzed: &mut AnalyzedStatement,
    mutate: impl FnOnce(&mut selene_gql::InlineProcedureCall),
) {
    let AnalyzedStatementKind::Query(query) = &mut analyzed.statement else {
        panic!("expected query statement");
    };
    for statement in &mut query.statements {
        if let PipelineStatement::CallSubquery(call) = statement {
            mutate(call);
            return;
        }
    }
    panic!("no CALL subquery to mutate");
}

fn assert_not_implemented(error: PlannerError, expected_feature: &str) {
    let PlannerError::NotImplemented { feature, .. } = error else {
        panic!("expected NotImplemented(\"{expected_feature}\"), got {error:?}");
    };
    assert_eq!(feature, expected_feature);
}

// ---------------------------------------------------------------------------
// PLAN-17: pattern-lowering defensive guards.
// ---------------------------------------------------------------------------

#[test]
fn empty_graph_pattern_is_rejected() {
    let mut analyzed = analyzed("MATCH (n) RETURN n");
    mutate_match_pattern(&mut analyzed, Vec::clear);
    assert_not_implemented(plan_err(&analyzed), "empty graph pattern");
}

#[test]
fn non_alternating_graph_pattern_is_rejected() {
    let mut analyzed = analyzed("MATCH (a)-[:E]->(b) RETURN a");
    // Make element[1] a Node (a copy of the leading node) so the walk hits a
    // node where an edge is required.
    mutate_match_pattern(&mut analyzed, |elements| {
        let node = elements[0].clone();
        elements[1] = node;
    });
    assert_not_implemented(plan_err(&analyzed), "non-alternating graph pattern");
}

#[test]
fn edge_without_target_is_rejected() {
    let mut analyzed = analyzed("MATCH (a)-[:E]->(b) RETURN a");
    // Drop the trailing target node so the pattern ends on an edge.
    mutate_match_pattern(&mut analyzed, |elements| {
        elements.pop();
    });
    assert_not_implemented(plan_err(&analyzed), "edge without target");
}

#[test]
fn different_edges_match_mode_is_rejected() {
    // PLAN-17: G003 (DIFFERENT EDGES) is absent from SUPPORTED_FEATURES, so the
    // flagger rejects it at parse time and `reject_unsupported_clause` never sees
    // a populated `match_mode` from real GQL. Set it directly on the analyzed
    // clause to drive the defense-in-depth guard and pin its exact tag.
    let mut analyzed = analyzed("MATCH (a)-[:E]->(b) RETURN a");
    mutate_match_clause(&mut analyzed, |clause| {
        clause.match_mode = Some(MatchMode::DifferentEdges);
    });
    assert_not_implemented(
        plan_err(&analyzed),
        "MATCH mode (REPEATABLE ELEMENTS / DIFFERENT EDGES)",
    );
}

#[test]
fn repeatable_elements_match_mode_is_rejected() {
    // PLAN-17: the G002 (REPEATABLE ELEMENTS) sibling of the test above. Both
    // MatchMode arms share the one rejection site, so this pins that neither
    // arm slips through if the guard is later loosened to handle only one.
    let mut analyzed = analyzed("MATCH (a)-[:E]->(b) RETURN a");
    mutate_match_clause(&mut analyzed, |clause| {
        clause.match_mode = Some(MatchMode::RepeatableElements);
    });
    assert_not_implemented(
        plan_err(&analyzed),
        "MATCH mode (REPEATABLE ELEMENTS / DIFFERENT EDGES)",
    );
}

// PLAN-17 coverage boundary: the questioned-edge / path-selector / path-mode
// "without … node/edge binding" guards (match_clause.rs::lower_match_clause,
// path_search.rs, path_mode.rs) are NOT reachable by black-box AST mutation.
// `node_scan` / `edge_match` always allocate a hidden binding when no named
// binding exists (`binding.is_none().then(|| hidden.next())`), so
// `chain_tail_binding` over a lowered scan always returns `Some`. Triggering
// those `None` arms requires hand-constructing the internal `JoinTree`, which
// is not a public API. They remain pure defense-in-depth; this file pins every
// tag that a parsed-then-analyzed AST can actually reach (empty /
// non-alternating / edge-without-target / MATCH-mode here; variable-scope CALL
// + IN TRANSACTIONS below; correlated NEXT + leading OPTIONAL MATCH are pinned
// in exec_pipeline_chain.rs + plan_read_pipeline.rs).

// ---------------------------------------------------------------------------
// PLAN-17: inline-CALL defensive guards.
// ---------------------------------------------------------------------------

#[test]
fn variable_scope_call_subquery_is_rejected() {
    let mut analyzed = analyzed("CALL { RETURN 1 AS x }");
    mutate_call_subquery(&mut analyzed, |call| {
        call.variable_scope = Some(vec![intern("x").expect("interns")]);
    });
    assert_not_implemented(
        plan_err(&analyzed),
        "explicit variable-scope CALL subquery (GP03)",
    );
}

#[test]
fn in_transactions_call_subquery_is_rejected() {
    let mut analyzed = analyzed("CALL { RETURN 1 AS x }");
    mutate_call_subquery(&mut analyzed, |call| {
        call.in_transactions = true;
    });
    assert_not_implemented(plan_err(&analyzed), "CALL { ... } IN TRANSACTIONS");
}

// ---------------------------------------------------------------------------
// PLAN-22: insert edge-endpoint resolution uses index.wrapping_sub(1).
// ---------------------------------------------------------------------------

#[test]
fn insert_pattern_starting_with_an_edge_is_a_write_set_pattern_mismatch() {
    // PLAN-22: edge endpoint resolution computes the left endpoint at
    // `index.wrapping_sub(1)`. The grammar forbids an INSERT pattern that starts
    // with an edge, so this defensive path (index 0 -> wrapping_sub(1) ==
    // usize::MAX -> elements.get(MAX) == None -> WriteSetPatternMismatch) has no
    // natural source. Build it by reordering a valid `INSERT (:A)-[:E]->(:B)` so
    // the edge sits at element index 0, keeping the write-set order in step.
    let mut analyzed = analyzed("INSERT (:A)-[:E]->(:B)");

    let AnalyzedStatementKind::Mutate(pipeline) = &mut analyzed.statement else {
        panic!("expected mutation pipeline");
    };
    for statement in pipeline.statements.iter_mut() {
        if let MutationStatement::Insert(insert) = statement {
            let elements = &mut insert.patterns[0].elements;
            let edge_index = elements
                .iter()
                .position(|element| matches!(element, PatternElement::Edge(_)))
                .expect("pattern has an edge");
            let edge = elements.remove(edge_index);
            elements.insert(0, edge);
        }
    }
    // Reorder the write-set entries so the InsertEdge entry is consumed first,
    // matching the new pattern-element walk order (the edge is now element 0).
    let write_set = analyzed.write_set.as_mut().expect("insert has a write set");
    let edge_entry = write_set
        .entries
        .iter()
        .position(|entry| matches!(entry.kind, WriteKind::InsertEdge { .. }))
        .expect("write set has an InsertEdge entry");
    let edge = write_set.entries.remove(edge_entry);
    write_set.entries.insert(0, edge);

    let error = plan_err(&analyzed);
    assert!(
        matches!(error, PlannerError::WriteSetPatternMismatch { .. }),
        "edge-at-index-0 must resolve its missing left endpoint to a \
         WriteSetPatternMismatch, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// PLAN-23: lower_call_subquery yield-resolution metadata mismatch.
// ---------------------------------------------------------------------------

#[test]
fn call_subquery_yield_of_unknown_column_is_a_metadata_mismatch() {
    // PLAN-23: when an inline CALL yields a column absent from its body schema,
    // `body_column` raises ProcedureMetadataMismatch with an EMPTY `procedure`
    // box. The empty box is the intentional inline-call convention (an inline
    // CALL has no named procedure to report, mirroring UnknownYieldColumn). The
    // grammar shape that triggers this can't be reached directly, so mutate the
    // analyzed CALL's yield to reference a non-existent column.
    let mut analyzed = analyzed("CALL { RETURN 1 AS x }");
    mutate_call_subquery(&mut analyzed, |call| {
        let span = call.span;
        call.yield_items = vec![YieldItem {
            column: YieldColumn::Named(intern("missing_column").expect("interns")),
            alias: None,
            span,
        }];
    });

    let error = plan_err(&analyzed);
    let PlannerError::ProcedureMetadataMismatch {
        procedure, detail, ..
    } = error
    else {
        panic!("expected ProcedureMetadataMismatch, got {error:?}");
    };
    assert!(
        procedure.is_empty(),
        "inline CALL has no named procedure; the box must be empty by convention"
    );
    assert_eq!(
        detail,
        "CALL subquery yield column missing from body output schema"
    );
}
