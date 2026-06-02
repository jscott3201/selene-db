//! ISO/IEC 39075:2024 §16.4 match-mode synonym surface (FU-3).
//!
//! The full `<match mode>` synonym set — EDGE / RELATIONSHIP / EDGES /
//! RELATIONSHIPS / ELEMENT / ELEMENTS, each optionally followed by BINDINGS
//! after the SINGULAR noun — all collapse onto the two `MatchMode` semantics
//! (`DIFFERENT EDGES` = G002, `REPEATABLE ELEMENTS` = G003). These tests live
//! in their own binary (split out of `flagger.rs`) to stay under the 700-LOC
//! file cap.

use selene_core::{GraphId, feature_register::FeatureId};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, MatchMode,
    PipelineStatement, Statement, TxContext, analyze, execute_pattern, execute_pipeline,
    feature_walk, parse, plan,
};
use selene_graph::SharedGraph;

/// Parse `source` and return the leading MATCH clause's `match_mode`, panicking
/// if `source` does not parse or contains no MATCH clause. Used to assert the
/// resolved [`MatchMode`] variant a synonym spelling collapses onto.
fn match_mode_of(source: &str) -> Option<MatchMode> {
    let statement = parse(source).unwrap_or_else(|e| panic!("{source:?} must parse: {e:?}"));
    let Statement::Query(pipeline) = statement else {
        panic!("{source:?} did not parse as a read query");
    };
    for stmt in &pipeline.statements {
        if let PipelineStatement::Match(clause) = stmt {
            return clause.match_mode;
        }
    }
    panic!("{source:?} has no MATCH clause");
}

/// Plan `source` as a read query, panicking on any parse/analyze/plan error.
fn read_plan(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect(source);
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect(source)
}

/// Assert `source` plans without error.
fn assert_read_plan(source: &str) {
    let _ = read_plan(source);
}

/// Assert `source` plans and executes without error against an empty graph.
fn assert_read_execution(source: &str) {
    let plan = read_plan(source);
    let graph = SharedGraph::new(GraphId::new(9151));
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx).expect(source)
    } else {
        BindingTable::new(
            BindingTableSchema {
                columns: Vec::new(),
            },
            vec![Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx).expect(source);
}

#[test]
fn match_mode_synonyms_resolve_to_two_variants() {
    // ISO/IEC 39075:2024 §16.4: <match mode> admits the noun synonyms EDGE /
    // RELATIONSHIP / EDGES / RELATIONSHIPS / ELEMENT / ELEMENTS, each optionally
    // followed by BINDINGS after the SINGULAR noun only. All edge-family
    // spellings are DIFFERENT EDGES (G002); all element-family spellings are
    // REPEATABLE ELEMENTS (G003) — pure syntactic sugar onto the two semantics.
    for source in [
        "MATCH DIFFERENT EDGE (n) RETURN n",
        "MATCH DIFFERENT EDGES (n) RETURN n",
        "MATCH DIFFERENT RELATIONSHIP (n) RETURN n",
        "MATCH DIFFERENT RELATIONSHIPS (n) RETURN n",
        "MATCH DIFFERENT EDGE BINDINGS (n) RETURN n",
        "MATCH DIFFERENT RELATIONSHIP BINDINGS (n) RETURN n",
    ] {
        assert_eq!(
            match_mode_of(source),
            Some(MatchMode::DifferentEdges),
            "{source:?} must resolve to DifferentEdges"
        );
        // Every synonym must still flag G002 and plan/execute unchanged.
        let features = feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            features.contains(&FeatureId::G002),
            "{source:?} must record G002; observed {features:?}"
        );
        assert_read_plan(source);
        assert_read_execution(source);
    }

    for source in [
        "MATCH REPEATABLE ELEMENT (n) RETURN n",
        "MATCH REPEATABLE ELEMENTS (n) RETURN n",
        "MATCH REPEATABLE ELEMENT BINDINGS (n) RETURN n",
    ] {
        assert_eq!(
            match_mode_of(source),
            Some(MatchMode::RepeatableElements),
            "{source:?} must resolve to RepeatableElements"
        );
        let features = feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            features.contains(&FeatureId::G003),
            "{source:?} must record G003; observed {features:?}"
        );
        assert_read_plan(source);
        assert_read_execution(source);
    }
}

#[test]
fn match_mode_rejects_plural_bindings_and_non_synonyms() {
    // ISO §16.4: BINDINGS is permitted ONLY after the SINGULAR noun. The plural
    // forms (ELEMENTS / EDGES / RELATIONSHIPS) admit no trailing BINDINGS, and
    // VERTEX / NODE are not ISO synonyms. A bare `DIFFERENT BINDINGS` is missing
    // the required edge noun. Each must be a parse error (the trailing tokens
    // can never start a valid graph pattern).
    for source in [
        "MATCH DIFFERENT EDGES BINDINGS (n) RETURN n",
        "MATCH DIFFERENT RELATIONSHIPS BINDINGS (n) RETURN n",
        "MATCH REPEATABLE ELEMENTS BINDINGS (n) RETURN n",
        "MATCH DIFFERENT VERTEX (n) RETURN n",
        "MATCH REPEATABLE NODE (n) RETURN n",
        "MATCH DIFFERENT BINDINGS (n) RETURN n",
    ] {
        assert!(
            parse(source).is_err(),
            "{source:?} is not a valid ISO §16.4 match mode and must be rejected"
        );
    }
}

#[test]
fn match_mode_synonyms_remain_usable_as_identifiers() {
    // The §16.4 synonym nouns are recognised CONTEXTUALLY (only in the match-mode
    // leading position) and are deliberately NOT added to the global `keyword`
    // rule (parser-DoS no-new-reserved-word posture, IMPLIES precedent 813). They
    // must therefore stay usable as ordinary property/variable identifiers.
    for source in [
        "MATCH (n) RETURN n.edge",
        "MATCH (n) RETURN n.edges",
        "MATCH (n) RETURN n.element",
        "MATCH (n) RETURN n.elements",
        "MATCH (n) RETURN n.relationship",
        "MATCH (n) RETURN n.relationships",
        "MATCH (n) RETURN n.bindings",
        "MATCH (n) RETURN n.repeatable",
        "MATCH (element) RETURN element",
        "MATCH (relationship) RETURN relationship",
        "MATCH (repeatable) RETURN repeatable",
        "MATCH (bindings) RETURN bindings",
    ] {
        parse(source).unwrap_or_else(|e| {
            panic!("{source:?} must parse: synonym nouns stay usable as identifiers: {e:?}")
        });
    }
}

#[test]
fn match_mode_leading_keyword_requires_token_separation() {
    // Codex review (PR #243, P2): pest's `~` separator is zero-or-more whitespace,
    // so without a word boundary on the LEADING mode keyword a run-together
    // spelling would parse as mode + synonym ("DIFFERENT" + "EDGE"). DIFFERENT /
    // REPEATABLE are boundary-guarded (different_mode_kw / repeatable_mode_kw), so
    // these concatenations are NOT ISO match modes and must be rejected — they can
    // only be an identifier, which cannot start a graph pattern here.
    for source in [
        "MATCH DIFFERENTEDGE (n) RETURN n",
        "MATCH DIFFERENTEDGES (n) RETURN n",
        "MATCH REPEATABLEELEMENT (n) RETURN n",
        "MATCH REPEATABLEELEMENTS (n) RETURN n",
    ] {
        assert!(
            parse(source).is_err(),
            "{source:?} runs the mode keyword into the noun and must be rejected"
        );
    }
}

#[test]
fn match_mode_bindings_defers_to_path_variable_named_bindings() {
    // Codex review (PR #243, P2): BINDINGS stays non-reserved, so a path variable
    // literally named `bindings` after a SINGULAR match mode must win over the
    // optional match-mode BINDINGS keyword. The `!("=")` guard releases `bindings`
    // to path_var_binding (`ident ~ "="`); the mode is still recognised. Had the
    // optional BINDINGS stolen `bindings`, the trailing `= (n)` could not start a
    // graph pattern and `match_mode_of` would panic on the parse error.
    for (source, expected) in [
        (
            "MATCH DIFFERENT EDGE bindings = (n) RETURN bindings",
            MatchMode::DifferentEdges,
        ),
        (
            "MATCH DIFFERENT RELATIONSHIP bindings = (n) RETURN bindings",
            MatchMode::DifferentEdges,
        ),
        (
            "MATCH REPEATABLE ELEMENT bindings = (n) RETURN bindings",
            MatchMode::RepeatableElements,
        ),
    ] {
        assert_eq!(
            match_mode_of(source),
            Some(expected),
            "{source:?} must parse with `bindings` as the path variable, mode intact"
        );
    }

    // The genuine match-mode BINDINGS (NOT followed by `=`) is still consumed.
    assert_eq!(
        match_mode_of("MATCH DIFFERENT EDGE BINDINGS (n) RETURN n"),
        Some(MatchMode::DifferentEdges),
        "DIFFERENT EDGE BINDINGS (...) must still parse as the bindings match mode"
    );
}

#[test]
fn match_mode_tolerates_comments_flush_against_keywords() {
    // Codex review (PR #243, P3): pest's `~` consumes implicit COMMENT as well as
    // whitespace, so a block comment may sit flush against the mode keyword (or the
    // noun) with NO surrounding space. `build_match_mode` dispatches on the first
    // child *rule* (`different_mode_kw` / `repeatable_mode_kw`), not a raw
    // `split_whitespace()` of the source span, so the un-delimited blob no longer
    // breaks dispatch. This also covers the plural forms, which had the same latent
    // failure pre-FU-3 (the existing comment test only used space-surrounded comments).
    for (source, expected) in [
        (
            "MATCH DIFFERENT/*x*/EDGE (n) RETURN n",
            MatchMode::DifferentEdges,
        ),
        (
            "MATCH DIFFERENT/*x*/EDGES (n) RETURN n",
            MatchMode::DifferentEdges,
        ),
        (
            "MATCH DIFFERENT/*x*/RELATIONSHIP (n) RETURN n",
            MatchMode::DifferentEdges,
        ),
        (
            "MATCH REPEATABLE/*x*/ELEMENT (n) RETURN n",
            MatchMode::RepeatableElements,
        ),
        (
            "MATCH REPEATABLE/*x*/ELEMENTS (n) RETURN n",
            MatchMode::RepeatableElements,
        ),
        (
            "MATCH DIFFERENT EDGE/*y*/BINDINGS (n) RETURN n",
            MatchMode::DifferentEdges,
        ),
    ] {
        assert_eq!(
            match_mode_of(source),
            Some(expected),
            "{source:?}: a comment flush against the keyword must not break dispatch"
        );
    }
}
