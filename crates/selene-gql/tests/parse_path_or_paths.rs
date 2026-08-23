//! Parser / flagger / runtime-inertness coverage for ISO/IEC 39075:2024 §16.6
//! `<path or paths>` — the explicit `PATH` / `PATHS` keyword (Feature G014,
//! "Explicit PATH/PATHS keywords").
//!
//! G014 is pure surface sugar per ISO §1.2.4: the keyword is parsed, the AST
//! `MatchClause.path_or_paths` flag is set, the flagger stamps G014, and the
//! runtime treats it as inert (the lowered plan is byte-identical to the
//! no-keyword spelling). These tests pin every one of those four facts.

use selene_core::feature_register::{FeatureId, SUPPORTED_FEATURES};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ParserError, PipelineStatement, Statement, analyze,
    feature_walk, parse, plan,
};

/// Parse `source`, returning the `MatchClause.path_or_paths` flag of the first
/// pipeline statement.
fn match_path_or_paths(source: &str) -> bool {
    let Statement::Query(query) = parse(source).expect("parse succeeds") else {
        panic!("expected query statement for {source:?}");
    };
    let PipelineStatement::Match(match_clause) = &query.statements[0] else {
        panic!("expected leading MATCH for {source:?}");
    };
    match_clause.path_or_paths
}

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("parse succeeds");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("analyze succeeds");
    plan(&analyzed, &EmptyProcedureRegistry).expect("plan succeeds")
}

fn observes_g014(source: &str) -> bool {
    let statement = parse(source).expect("parse succeeds");
    feature_walk(&statement)
        .into_iter()
        .any(|feature| feature.feature_id == FeatureId::G014)
}

#[test]
fn g014_is_runtime_supported() {
    assert!(
        SUPPORTED_FEATURES.contains(&FeatureId::G014),
        "G014 (Explicit PATH/PATHS keywords) must be in SUPPORTED_FEATURES"
    );
}

#[test]
fn explicit_path_or_paths_parses_and_sets_flag() {
    // Every ISO §16.6 form where <path or paths> is reachable through selene's
    // flattened `path_selector? ~ match_mode? ~ path_modifier? ~ path_or_paths?`
    // prefix. <path or paths> is the trailing token in each case.
    for source in [
        // <all path search> ::= ALL [ <path mode> ] [ <path or paths> ]
        "MATCH ALL PATHS (n) RETURN n",
        "MATCH ALL PATH (n) RETURN n",
        // <any path search> ::= ANY [ ... ] [ <path or paths> ]
        "MATCH ANY PATHS (n) RETURN n",
        // <all shortest path search> ::= ALL SHORTEST [ <path mode> ] [ <path or paths> ]
        "MATCH ALL SHORTEST PATHS (n)-[:K]->(m) RETURN m",
        // <any shortest path search> ::= ANY SHORTEST [ <path mode> ] [ <path or paths> ]
        "MATCH ANY SHORTEST PATH (n)-[:K]->(m) RETURN m",
        // <counted shortest path search> ::= SHORTEST <n> [ <path mode> ] [ <path or paths> ]
        "MATCH SHORTEST 2 PATHS (n)-[:K]->(m) RETURN m",
        // <path mode prefix> ::= <path mode> [ <path or paths> ] — all four modes,
        // both the plural and singular <path or paths> spelling.
        "MATCH WALK PATHS (n) RETURN n",
        "MATCH WALK PATH (n) RETURN n",
        "MATCH TRAIL PATH (n)-[:K]->(m) RETURN m",
        "MATCH SIMPLE PATHS (n)-[:K]->(m) RETURN m",
        "MATCH ACYCLIC PATH (n)-[:K]->(m) RETURN m",
    ] {
        assert!(
            match_path_or_paths(source),
            "{source:?} must set MatchClause.path_or_paths = true"
        );
    }
}

#[test]
fn bare_path_or_paths_without_mode_or_selector_is_rejected() {
    // ISO §16.6: <path or paths> is never standalone — it is the optional trailing
    // token of a <path mode prefix> (which REQUIRES a <path mode>) or a
    // <path search prefix> (ALL/ANY/SHORTEST). selene's flattened grammar makes the
    // prefix pieces independently optional, so the builder rejects a bare PATH/PATHS
    // with neither a selector nor an explicit mode. A <match mode> (§16.4 DIFFERENT
    // EDGES) is NOT a §16.6 prefix and does not satisfy the requirement.
    for source in [
        "MATCH PATHS (n) RETURN n",
        "MATCH PATH (n) RETURN n",
        "MATCH DIFFERENT EDGES PATHS (n) RETURN n",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn path_or_paths_is_accepted_in_a_data_modifying_match() {
    // The §16.6 path-pattern-prefix (selector / mode / <path or paths>) belongs to
    // the MATCH clause, which ISO permits before a data-modifying statement (the
    // MATCH binds the rows the mutation operates on). So PATH/PATHS in a MATCH that
    // precedes DELETE/SET is conforming — exactly as the pre-existing ALL/ANY/
    // SHORTEST selector already is. (Guards against a false "data-modifying
    // over-acceptance" reading: match_stmt is the shared MATCH clause, not a
    // construct embedded inside DELETE itself.)
    for source in [
        "MATCH ALL PATHS (n) DELETE n",
        "MATCH WALK PATHS (n) SET n.age = 1",
    ] {
        parse(source).unwrap_or_else(|err| {
            panic!(
                "{source:?} must parse: PATH/PATHS in a mutation MATCH is conforming, got {err:?}"
            )
        });
    }
}

#[test]
fn path_keywords_require_delimited_path_variables() {
    // PATH/PATHS are ISO §21.3 reserved words. They remain legal path-variable
    // names only when delimited, and the G014 surface flag stays false because
    // the quoted identifier is a binding name rather than the optional keyword.
    for source in [
        "MATCH paths = (n) RETURN paths",
        "MATCH path = (n) RETURN path",
        "MATCH ALL paths = (n) RETURN paths",
        "MATCH WALK path = (n) RETURN path",
        "MATCH SHORTEST 2 paths = (n)-[:K]->(m) RETURN paths",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }

    for source in [
        "MATCH \"paths\" = (n) RETURN \"paths\"",
        "MATCH \"path\" = (n) RETURN \"path\"",
        "MATCH ALL \"paths\" = (n) RETURN \"paths\"",
        "MATCH WALK \"path\" = (n) RETURN \"path\"",
        "MATCH SHORTEST 2 \"paths\" = (n)-[:K]->(m) RETURN \"paths\"",
    ] {
        assert!(
            !match_path_or_paths(source),
            "{source:?} must parse with a delimited path variable, G014 flag false"
        );
    }
}

#[test]
fn counted_group_accepts_path_or_paths_in_iso_order() {
    // Codex (PR #244, P2): ISO §16.6 <counted shortest group search> is
    // `SHORTEST [n] [mode] [<path or paths>] {GROUP|GROUPS}` — <path or paths> comes
    // BEFORE the group discriminator. That conforming spelling is parsed inside
    // counted_shortest_tail and sets the G014 flag (combining the runtime-supported G014 +
    // G020 surfaces).
    for source in [
        "MATCH SHORTEST 2 PATHS GROUPS (a)-[:K]->(b) RETURN b",
        "MATCH SHORTEST PATHS GROUPS (a)-[:K]->(b) RETURN b",
        "MATCH SHORTEST 1 PATH GROUP (a)-[:K]->(b) RETURN b",
    ] {
        assert!(
            match_path_or_paths(source),
            "{source:?} (ISO order PATHS before GROUP[S]) must parse and set G014"
        );
        assert!(observes_g014(source), "{source:?} must flag G014");
    }
}

#[test]
fn path_or_paths_rejected_after_counted_group_in_wrong_order() {
    // The non-ISO order — <path or paths> AFTER the GROUP/GROUPS discriminator
    // (`SHORTEST n GROUPS PATHS`) — reaches the trailing slot and is rejected. The
    // conforming spelling is `SHORTEST n PATHS GROUPS` (see the test above).
    for source in [
        "MATCH SHORTEST 2 GROUPS PATHS (a)-[:K]->(b) RETURN b",
        "MATCH SHORTEST GROUPS PATHS (a)-[:K]->(b) RETURN b",
        "MATCH SHORTEST 1 GROUP PATH (a)-[:K]->(b) RETURN b",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn match_mode_may_not_separate_path_or_paths_from_the_prefix() {
    // Codex (PR #244, P2): ISO §16.6 binds <path or paths> to the path prefix; the
    // §16.4 <match mode> (DIFFERENT EDGES / REPEATABLE ELEMENTS) is a separate,
    // graph-level construct and cannot sit between the prefix and PATH/PATHS.
    // Both carry a real selector, so the bare-prefix guard does not fire — the
    // match-mode interposition guard is what rejects them.
    for source in [
        "MATCH ALL DIFFERENT EDGES PATHS (n) RETURN n",
        "MATCH ANY REPEATABLE ELEMENTS PATH (n) RETURN n",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn concatenated_prefix_and_path_or_paths_is_rejected() {
    // Codex (PR #244, P2): pest's `~` separator is zero-or-more whitespace, so the
    // leading selector / mode keywords must be boundary-guarded or a run-together
    // spelling would tokenize as keyword + PATHS. The boundary-guarded atomic
    // sub-rules (all_kw / any_kw / shortest_kw and the atomic path_modifier) make
    // these reject.
    for source in [
        "MATCH ALLPATHS (n) RETURN n",
        "MATCH ANYPATHS (n) RETURN n",
        "MATCH WALKPATHS (n) RETURN n",
        "MATCH ACYCLICPATH (n) RETURN n",
        "MATCH SIMPLEPATHS (n) RETURN n",
        "MATCH TRAILPATH (n) RETURN n",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn absent_path_or_paths_leaves_flag_false() {
    for source in [
        "MATCH (n) RETURN n",
        "MATCH ALL (n) RETURN n",
        "MATCH ANY SHORTEST (n)-[:K]->(m) RETURN m",
        "MATCH SHORTEST 2 (n)-[:K]->(m) RETURN m",
        "MATCH WALK (n) RETURN n",
    ] {
        assert!(
            !match_path_or_paths(source),
            "{source:?} must leave MatchClause.path_or_paths = false"
        );
    }
}

#[test]
fn hand_trace_shortest_2_paths() {
    // Brief hand-trace: `SHORTEST 2` is consumed by path_selector
    // (counted_shortest_tail = uint), path_modifier? is empty, and the trailing
    // path_or_paths? eats "PATHS". PATH/PATHS is NOT inside counted_shortest_tail.
    assert!(match_path_or_paths(
        "MATCH SHORTEST 2 PATHS (n)-[:K]->(m) RETURN m"
    ));
    // The no-keyword spelling is the same counted-shortest selector, flag false.
    assert!(!match_path_or_paths(
        "MATCH SHORTEST 2 (n)-[:K]->(m) RETURN m"
    ));
}

#[test]
fn flagger_records_g014_iff_keyword_present() {
    // With PATHS -> G014 observed.
    assert!(observes_g014("MATCH ALL PATHS (n) RETURN n"));
    assert!(observes_g014("MATCH WALK PATHS (n) RETURN n"));
    // The SAME query without PATH/PATHS -> G014 NOT observed.
    assert!(!observes_g014("MATCH ALL (n) RETURN n"));
    assert!(!observes_g014("MATCH WALK (n) RETURN n"));
    assert!(!observes_g014("MATCH (n) RETURN n"));
}

/// Erase span / fingerprint noise from a plan's `Debug` string so the
/// comparison is structural. The explicit PATH/PATHS keyword shifts downstream
/// byte offsets (and the span-derived expr fingerprint), but those are source
/// metadata, not plan structure — stripping them leaves the structural plan.
fn structural_plan(source: &str) -> String {
    let debug = format!("{:?}", planned(source));
    let mut out = String::with_capacity(debug.len());
    let mut chars = debug.char_indices().peekable();
    while let Some((idx, _)) = chars.peek().copied() {
        if debug[idx..].starts_with("SourceSpan { ") {
            // Skip to the matching close brace.
            for (_, c) in chars.by_ref() {
                if c == '}' {
                    break;
                }
            }
            out.push_str("SourceSpan");
        } else if debug[idx..].starts_with("fingerprint: ") {
            // Skip "fingerprint: <digits>".
            for _ in 0.."fingerprint: ".len() {
                chars.next();
            }
            while let Some((_, c)) = chars.peek().copied() {
                if c.is_ascii_digit() {
                    chars.next();
                } else {
                    break;
                }
            }
            out.push_str("fingerprint");
        } else {
            out.push(chars.next().expect("peeked char exists").1);
        }
    }
    out
}

#[test]
fn path_or_paths_is_runtime_inert() {
    // ISO §1.2.4: the explicit keyword carries no semantic effect. The lowered
    // ExecutionPlan for the PATH/PATHS spelling must be structurally identical
    // to the no-keyword spelling (only source spans / span fingerprints differ,
    // because the keyword shifts byte offsets) — proving the runtime treats it
    // as inert. There is no G014 node, predicate, or selector difference in the
    // plan tree.
    for (with_kw, without_kw) in [
        ("MATCH ALL PATHS (n) RETURN n", "MATCH ALL (n) RETURN n"),
        ("MATCH WALK PATHS (n) RETURN n", "MATCH WALK (n) RETURN n"),
        (
            "MATCH SHORTEST 2 PATHS (n)-[:K]->(m) RETURN m",
            "MATCH SHORTEST 2 (n)-[:K]->(m) RETURN m",
        ),
        (
            "MATCH ANY SHORTEST PATH (n)-[:K]->(m) RETURN m",
            "MATCH ANY SHORTEST (n)-[:K]->(m) RETURN m",
        ),
    ] {
        assert_eq!(
            structural_plan(with_kw),
            structural_plan(without_kw),
            "{with_kw:?} must plan identically to {without_kw:?} (G014 is inert)"
        );
    }
}

#[test]
fn misplaced_path_or_paths_is_rejected() {
    // <path or paths> is the TRAILING token of the §16.6 prefix. PATHS before the
    // selector, or before the count, is not a legal position and must reject.
    for source in [
        // PATHS before the selector keyword.
        "MATCH PATHS ALL (n) RETURN n",
        // PATHS before the count.
        "MATCH SHORTEST PATHS 2 (n)-[:K]->(m) RETURN m",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn path_keywords_stay_available_for_properties_and_delimited_identifiers() {
    // Property keys use `prop_ident`, so keyword-shaped property names remain
    // available. Binding and alias positions must use delimited identifiers.
    for source in [
        "MATCH (n) RETURN n.path",
        "MATCH (n) RETURN n.paths",
        "MATCH (\"path\") RETURN \"path\"",
        "MATCH (\"paths\") RETURN \"paths\"",
    ] {
        parse(source).unwrap_or_else(|err| {
            panic!("{source:?} must parse with reserved PATH/PATHS usage, got {err:?}")
        });
    }

    for source in [
        "MATCH (path) RETURN path",
        "MATCH (paths) RETURN paths",
        "RETURN 1 AS path",
        "RETURN 1 AS paths",
    ] {
        let err = parse(source).expect_err(&format!("{source:?} must be rejected"));
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}
