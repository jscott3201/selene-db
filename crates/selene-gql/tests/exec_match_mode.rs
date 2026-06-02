//! Executor match-mode tests (ISO/IEC 39075:2024 §16.4).
//!
//! Covers Feature G002 (DIFFERENT EDGES) pattern-wide edge-uniqueness filtering
//! and Feature G003 (REPEATABLE ELEMENTS) / the ID086 default (= REPEATABLE
//! ELEMENTS) installing no filter. A match-mode violation FILTERS (drops the
//! row); §16.4 attaches no GQLSTATUS, so no error is raised.

mod exec_common;

use exec_common::{istr, planned, props};
use selene_core::{GraphId, LabelSet, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutorError, TxContext,
    execute_pattern, execute_pipeline,
};
use selene_graph::SharedGraph;

/// Graph: A, B, C nodes (each `name`-tagged) joined by single directed `K`
/// edges A->B, B->C, A->C, plus one A->A self-loop. The single physical edge
/// per node pair lets two distinct edge variables collide on one `EdgeId`,
/// which is the GR4 violation a DIFFERENT EDGES filter must catch pattern-wide.
struct MatchModeFixture {
    graph: SharedGraph,
}

impl MatchModeFixture {
    fn build() -> Self {
        let node = istr("N");
        let edge = istr("K");
        let name = istr("name");
        let graph = SharedGraph::new(GraphId::new(7421));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("A")))]),
                )
                .expect("A inserts");
            let b = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("B")))]),
                )
                .expect("B inserts");
            let c = mutator
                .create_node(
                    LabelSet::single(node),
                    props([(name, Value::String(istr("C")))]),
                )
                .expect("C inserts");
            mutator
                .create_edge(edge.clone(), a, a, props([]))
                .expect("A self-loop");
            mutator
                .create_edge(edge.clone(), a, b, props([]))
                .expect("A to B");
            mutator
                .create_edge(edge.clone(), b, c, props([]))
                .expect("B to C");
            mutator.create_edge(edge, a, c, props([])).expect("A to C");
            txn.commit().expect("fixture commits");
        }
        Self { graph }
    }

    fn execute(&self, source: &str) -> Result<BindingTable, ExecutorError> {
        execute_on(&self.graph, source)
    }

    fn row_count(&self, source: &str) -> usize {
        self.execute(source).expect("query executes").row_count()
    }
}

/// Plan and execute `source` against `graph`, returning the raw binding table.
/// Shared by every match-mode fixture so they need not duplicate the read-only
/// `TxContext` wiring.
fn execute_on(graph: &SharedGraph, source: &str) -> Result<BindingTable, ExecutorError> {
    let plan = planned(source);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx)?
    } else {
        BindingTable::new(
            BindingTableSchema {
                columns: Vec::new(),
            },
            vec![Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx)
}

#[test]
fn different_edges_drops_edge_reused_across_comma_separated_patterns() {
    // ISO §16.4 GR4 / GR8(a): the `<match mode>` is pattern-wide across the
    // comma-separated path-pattern list. Two edge variables (`e1`, `e2`) over
    // the single physical A->B edge bind the same `EdgeId` at two positions, so
    // DIFFERENT EDGES drops the row; REPEATABLE ELEMENTS keeps it; the
    // no-prefix default (ID086 = REPEATABLE ELEMENTS) keeps it too.
    let fixture = MatchModeFixture::build();
    let pattern = "(a:N {name: 'A'})-[e1:K]->(b:N {name: 'B'}), (a)-[e2:K]->(b)";

    assert_eq!(
        fixture.row_count(&format!("MATCH DIFFERENT EDGES {pattern} RETURN a")),
        0,
        "DIFFERENT EDGES must drop the row that reuses A->B across both patterns"
    );
    assert_eq!(
        fixture.row_count(&format!("MATCH REPEATABLE ELEMENTS {pattern} RETURN a")),
        1,
        "REPEATABLE ELEMENTS must keep the repeated-edge row"
    );
    assert_eq!(
        fixture.row_count(&format!("MATCH {pattern} RETURN a")),
        1,
        "the no-prefix default (ID086 = REPEATABLE ELEMENTS) must match REPEATABLE ELEMENTS"
    );
}

#[test]
fn default_match_mode_equals_explicit_repeatable_elements() {
    // Pins ID086 = REPEATABLE ELEMENTS: a no-prefix MATCH over the
    // edge-reuse fixture returns exactly the rows explicit REPEATABLE ELEMENTS
    // returns (and strictly more than DIFFERENT EDGES) — no silent break to
    // existing queries.
    let fixture = MatchModeFixture::build();
    let pattern = "(a:N {name: 'A'})-[e1:K]->(b:N {name: 'B'}), (a)-[e2:K]->(b)";

    let default_rows = fixture.row_count(&format!("MATCH {pattern} RETURN a"));
    let repeatable_rows =
        fixture.row_count(&format!("MATCH REPEATABLE ELEMENTS {pattern} RETURN a"));
    let different_rows = fixture.row_count(&format!("MATCH DIFFERENT EDGES {pattern} RETURN a"));

    assert_eq!(default_rows, repeatable_rows);
    assert!(default_rows > different_rows);
}

#[test]
fn different_edges_drops_two_hop_chain_reusing_self_loop() {
    // ISO §16.4 GR4 within a single path pattern: a 2-hop fixed chain over the
    // A->A self-loop reuses one `EdgeId` at two positions, so DIFFERENT EDGES
    // drops it; WALK (no match mode, no path mode) keeps both traversals.
    let fixture = MatchModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH DIFFERENT EDGES (a:N {name: 'A'})-[:K]->(m:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        0,
        "DIFFERENT EDGES must drop the self-loop reused twice"
    );
    assert_eq!(
        fixture.row_count(
            "MATCH (a:N {name: 'A'})-[:K]->(m:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        1,
        "the no-prefix default keeps the repeated self-loop"
    );
}

#[test]
fn different_edges_dedups_inside_variable_length_group() {
    // ISO §16.4 GR4 over a `*` group column: a 2-hop variable-length segment
    // forced onto the A->A self-loop reuses one `EdgeId` inside the group's
    // `LIST<EdgeRef>`, so DIFFERENT EDGES drops it; WALK keeps it.
    let fixture = MatchModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH DIFFERENT EDGES (a:N {name: 'A'})-[r:K*2..2]->(b:N {name: 'A'}) RETURN b"
        ),
        0,
        "DIFFERENT EDGES must dedup the self-loop reused inside the group"
    );
    assert_eq!(
        fixture.row_count("MATCH WALK (a:N {name: 'A'})-[r:K*2..2]->(b:N {name: 'A'}) RETURN b"),
        1,
        "WALK keeps the self-loop reused inside the group"
    );
}

#[test]
fn different_edges_multiply_declared_edge_variable_yields_empty() {
    // ISO §16.4 NOTE 225 / NOTE 227: a multiply-declared edge variable within a
    // graph pattern produces no matches under DIFFERENT EDGES — `()-[E]->()
    // -[E]->()` binds the same edge variable `E` to two positions, which the
    // pattern-wide filter rejects (empty result, not an error).
    let fixture = MatchModeFixture::build();

    assert_eq!(
        fixture.row_count("MATCH DIFFERENT EDGES ()-[e:K]->()-[e:K]->() RETURN 1"),
        0,
        "a multiply-declared edge variable under DIFFERENT EDGES yields empty"
    );
}

#[test]
fn different_edges_keeps_rows_with_distinct_edges() {
    // Positive control: DIFFERENT EDGES does NOT over-filter. A two-hop chain
    // over two distinct physical edges (A->B then B->C) keeps the row.
    let fixture = MatchModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH DIFFERENT EDGES (a:N {name: 'A'})-[:K]->(b:N {name: 'B'})-[:K]->(c:N {name: 'C'}) RETURN c"
        ),
        1,
        "DIFFERENT EDGES keeps a chain over two distinct edges"
    );
}

#[test]
fn multiply_declared_edge_under_walk_matches_but_different_edges_drops() {
    // Confirms the NOTE-225 emptiness is produced BY the DIFFERENT EDGES filter,
    // not by the multiply-declared edge variable failing to match at all: under
    // WALK the self-equijoin `()-[e]->()-[e]->()` binds the same edge to both
    // positions and DOES match (>0 rows), while DIFFERENT EDGES drops them all.
    let fixture = MatchModeFixture::build();
    let walk = fixture.row_count("MATCH ()-[e:K]->()-[e:K]->() RETURN 1");
    let different = fixture.row_count("MATCH DIFFERENT EDGES ()-[e:K]->()-[e:K]->() RETURN 1");
    assert!(walk > 0, "WALK self-equijoin should match, got {walk}");
    assert_eq!(different, 0, "DIFFERENT EDGES must drop all such rows");
}

#[test]
fn pipeline_match_different_edges_filters_in_a_later_segment() {
    // A `<match mode>` on a non-leading MATCH (after `NEXT`) lowers through
    // `lower_pipeline_match` -> `lower_match_clause`, so the pattern-wide filter
    // installs there too: the self-loop 2-hop chain is dropped under DIFFERENT
    // EDGES, kept under the no-prefix default.
    let fixture = MatchModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH (seed:N {name: 'A'}) NEXT \
             MATCH DIFFERENT EDGES (a:N {name: 'A'})-[:K]->(m:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        0,
        "pipeline-segment DIFFERENT EDGES must drop the reused self-loop"
    );
    assert_eq!(
        fixture.row_count(
            "MATCH (seed:N {name: 'A'}) NEXT \
             MATCH (a:N {name: 'A'})-[:K]->(m:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        1,
        "pipeline-segment no-prefix default keeps the reused self-loop"
    );
}

#[test]
fn different_edges_anonymous_quantified_edge_plans_and_executes() {
    // Regression (Codex P1): under DIFFERENT EDGES an *anonymous* quantified
    // edge still needs an edge-identity group slot so the pattern-wide filter
    // can read its edges — ISO §16.4 NOTE 222 imparts TRAIL to each path
    // pattern. Before the fix this failed to plan ("path mode over quantified
    // edge without edge group slot") while the named form `[r:K*1..2]` worked,
    // leaving a legal G002 pattern unsupported right after claiming the feature.
    let fixture = MatchModeFixture::build();
    let anon = "MATCH DIFFERENT EDGES (a:N {name: 'A'})-[:K*1..2]->(b) RETURN b";
    let named = "MATCH DIFFERENT EDGES (a:N {name: 'A'})-[r:K*1..2]->(b) RETURN b";

    let anon_rows = fixture
        .execute(anon)
        .expect("anonymous quantified edge under DIFFERENT EDGES must plan + execute")
        .row_count();
    assert!(anon_rows > 0, "expected matches, got {anon_rows}");
    assert_eq!(
        anon_rows,
        fixture.row_count(named),
        "anonymous and named quantified edges must agree under DIFFERENT EDGES"
    );
}

#[test]
fn different_edges_unbounded_quantifier_terminates_on_cycle() {
    // Regression (Codex P1): the analyzer admits an unbounded quantifier gated
    // only by DIFFERENT EDGES (`validate_unbounded_legality`) on the ISO §16.4
    // NOTE-222 premise that DIFFERENT EDGES imparts TRAIL and so bounds the
    // traversal. The runtime must honour that DURING the repeat: over the A->A
    // self-loop a bare WALK repeat reuses the self-loop edge forever and raises
    // 5GQL1 ProgramLimitExceeded before the post-walk filter runs. With the fix
    // the repeat traverses as TRAIL and terminates. (A WALK control is
    // inexpressible — the analyzer rejects an unbounded WALK quantifier.)
    let fixture = MatchModeFixture::build();
    let different = "MATCH DIFFERENT EDGES (a:N {name: 'A'})-[r:K+]->(b) RETURN b";

    let rows = fixture
        .execute(different)
        .expect("unbounded DIFFERENT EDGES repeat must terminate, not ProgramLimitExceeded")
        .row_count();
    assert!(rows > 0, "expected finite matches, got {rows}");
    // NOTE 222: on a single path pattern, DIFFERENT EDGES coincides with TRAIL
    // (no other path pattern's edges to conflict with), so the row sets agree.
    assert_eq!(
        rows,
        fixture.row_count("MATCH TRAIL (a:N {name: 'A'})-[r:K+]->(b) RETURN b"),
        "single-path DIFFERENT EDGES must match TRAIL (NOTE 222)"
    );
}

#[test]
fn different_edges_anonymous_unbounded_quantifier_terminates() {
    // Regression (Codex P1, combined): an *anonymous* *unbounded* quantified
    // edge under DIFFERENT EDGES needs BOTH fixes at once — the hidden group
    // slot (so it plans) and TRAIL-during-traversal (so it terminates). Pins
    // that the two repeat-lowering changes compose for `[:K+]`.
    let fixture = MatchModeFixture::build();
    let rows = fixture
        .execute("MATCH DIFFERENT EDGES (a:N {name: 'A'})-[:K+]->(b) RETURN b")
        .expect("anonymous unbounded DIFFERENT EDGES repeat must plan + terminate")
        .row_count();
    assert!(rows > 0, "expected finite matches, got {rows}");
    assert_eq!(
        rows,
        fixture.row_count("MATCH DIFFERENT EDGES (a:N {name: 'A'})-[r:K+]->(b) RETURN b"),
        "anonymous and named unbounded forms must agree under DIFFERENT EDGES"
    );
}

/// Graph with a 2-cycle through *distinct* edges plus a self-loop: nodes A and
/// B (label `N`) joined by A->A (self-loop), A->B, and B->A. From A there are
/// two 2-hop paths back to A — A->A->A (reuses the self-loop edge) and
/// A->B->A (two distinct edges) — which lets a path selector exercise the
/// §16.4 NOTE-222 ordering requirement.
struct SelectorCycleFixture {
    graph: SharedGraph,
}

impl SelectorCycleFixture {
    fn build() -> Self {
        let node = istr("N");
        let edge = istr("K");
        let name = istr("name");
        let graph = SharedGraph::new(GraphId::new(7422));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("A")))]),
                )
                .expect("A inserts");
            let b = mutator
                .create_node(
                    LabelSet::single(node),
                    props([(name, Value::String(istr("B")))]),
                )
                .expect("B inserts");
            // Self-loop created FIRST so it is the lowest edge id and is
            // enumerated before A->B in adjacency order — a WALK repeat would
            // surface A->A->A to the selector first.
            mutator
                .create_edge(edge.clone(), a, a, props([]))
                .expect("A self-loop");
            mutator
                .create_edge(edge.clone(), a, b, props([]))
                .expect("A to B");
            mutator.create_edge(edge, b, a, props([])).expect("B to A");
            txn.commit().expect("fixture commits");
        }
        Self { graph }
    }

    fn row_count(&self, source: &str) -> usize {
        execute_on(&self.graph, source)
            .expect("query executes")
            .row_count()
    }
}

#[test]
fn different_edges_selector_ranks_only_edge_distinct_paths() {
    // Regression (Codex round-2 P1): a path selector (`ANY`/SHORTEST) wraps the
    // repeat and runs BEFORE the outer pattern-wide `MatchModeFilter`. If the
    // bounded repeat stayed WALK, `ANY` could pick the A->A->A walk that reuses
    // the self-loop, the filter would then drop it, and the valid edge-distinct
    // A->B->A path would be lost (0 rows). Per ISO §16.4 NOTE 222 the path
    // pattern itself behaves as TRAIL under DIFFERENT EDGES, so the selector
    // sees only edge-distinct paths and returns the valid one.
    let fixture = SelectorCycleFixture::build();
    let any_different =
        "MATCH ANY DIFFERENT EDGES (a:N {name: 'A'})-[:K*2..2]->(b:N {name: 'A'}) RETURN b";

    assert_eq!(
        fixture.row_count(any_different),
        1,
        "ANY DIFFERENT EDGES must surface the edge-distinct A->B->A path, not lose it to the filter"
    );
    // NOTE 222 equivalence: on a single path pattern, ANY DIFFERENT EDGES and
    // ANY TRAIL select from the same edge-distinct candidate set.
    assert_eq!(
        fixture.row_count(any_different),
        fixture
            .row_count("MATCH ANY TRAIL (a:N {name: 'A'})-[:K*2..2]->(b:N {name: 'A'}) RETURN b"),
        "ANY DIFFERENT EDGES must agree with ANY TRAIL on a single path pattern"
    );
}
