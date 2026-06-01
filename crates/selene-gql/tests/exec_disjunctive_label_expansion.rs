//! BRIEF-155 Commit 4 — end-to-end execution + composition tests for the
//! disjunctive-label-expansion optimizer rule.
//!
//! Validates:
//! - Row-set equivalence with the manual `UNION ALL` rewrite (the ariadne
//!   workaround) on a single-label fixture — acceptance bar #6. Multi-label
//!   nodes diverge from manual UNION ALL by design (see PR #177 C1 below);
//!   this test exercises the no-multi-label case where the two forms agree.
//! - Composition with BRIEF-154 parameterized index selection (acceptance
//!   bar #7).
//! - Composition with BRIEF-153 STRING-index ExternalString carve-out.
//! - Downstream LIMIT / ORDER BY / GROUP BY — the union happens at
//!   JoinTree level, so the pipeline operates on the unioned binding
//!   table, not per branch.
//! - Q11 / PR #177 C1: multi-label nodes dedup at the
//!   `JoinTree::DisjunctiveScan` executor arm so the rule-firing path
//!   matches the unexpanded `LabelExpr::Disjunction(any(...))` semantics
//!   (one row per node), preserving the catalog-present vs catalog-absent
//!   invariant for COUNT / LIMIT / aggregates.

use std::collections::BTreeMap;
use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, EmptyProcedureRegistry, ExecutionPlan, ExecutorError, OptimizeContext, Session,
    StatementOutput, analyze, execute_statement, optimize, parse, plan,
};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_testing::MockIndexCatalog;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn props<const N: usize>(pairs: [(IStr, Value); N]) -> PropertyMap {
    PropertyMap::from_pairs(pairs).expect("test properties fit caps")
}

/// Plan + analyze + optimize against a real catalog. Session::execute_source
/// does NOT call the optimizer (Session only runs `plan()`), so the rule
/// would not fire if we used `execute_source` directly. By optimizing here
/// and then calling `execute_statement(&plan, session, registry)` we get
/// both bound parameters (via Session) and rule firing (via the catalog
/// injected at optimize time).
fn optimized(source: &str, catalog: &MockIndexCatalog) -> ExecutionPlan {
    let statement = parse(source).expect("source parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("source analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("source plans");
    let ctx = OptimizeContext::default().with_index_catalog(catalog);
    optimize(plan, &ctx)
}

fn execute_optimized(
    session: &mut Session<'_>,
    plan: &ExecutionPlan,
) -> Result<StatementOutput, ExecutorError> {
    execute_statement(plan, session, &EmptyProcedureRegistry)
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn node_ref_ids(table: &BindingTable) -> Vec<u64> {
    table
        .rows()
        .iter()
        .filter_map(|row| match row.values().first() {
            Some(Value::NodeRef(id)) => Some(id.get()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fixture: 3 Persons, 2 Robots, 1 Alien (no Alien index)
// ---------------------------------------------------------------------------

struct LabelFamilyFixture {
    graph: SharedGraph,
    catalog: MockIndexCatalog,
}

impl LabelFamilyFixture {
    fn build() -> Self {
        let person = istr("Person");
        let robot = istr("Robot");
        let alien = istr("Alien");
        let email = istr("email");
        let age = istr("age");
        let department = istr("department");

        let graph = SharedGraph::new(GraphId::new(1550));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            for name in ["alice", "bob", "cara"] {
                let person_age = match name {
                    "alice" => 20,
                    "bob" => 25,
                    _ => 30,
                };
                mutator
                    .create_node(
                        LabelSet::single(person.clone()),
                        props([
                            (email.clone(), Value::String(istr(name))),
                            (age.clone(), Value::Int(person_age)),
                            (department.clone(), Value::String(istr("engineering"))),
                        ]),
                    )
                    .expect("Person inserts");
            }
            for (name, robot_age) in [("r2d2", 100), ("wall-e", 125)] {
                mutator
                    .create_node(
                        LabelSet::single(robot.clone()),
                        props([
                            (email.clone(), Value::String(istr(name))),
                            (age.clone(), Value::Int(robot_age)),
                            (department.clone(), Value::String(istr("engineering"))),
                        ]),
                    )
                    .expect("Robot inserts");
            }
            mutator
                .create_node(
                    LabelSet::single(alien),
                    props([
                        (email.clone(), Value::String(istr("zorblax"))),
                        (age.clone(), Value::Int(99_999)),
                        (department, Value::String(istr("xenobiology"))),
                    ]),
                )
                .expect("Alien inserts");
            txn.commit().expect("fixture commits");
        }
        graph
            .create_property_index(person.clone(), email.clone(), TypedIndexKind::String)
            .expect("Person.email index builds");
        graph
            .create_property_index(robot.clone(), email.clone(), TypedIndexKind::String)
            .expect("Robot.email index builds");
        graph
            .create_property_index(person.clone(), age.clone(), TypedIndexKind::I64)
            .expect("Person.age index builds");
        graph
            .create_property_index(robot.clone(), age.clone(), TypedIndexKind::I64)
            .expect("Robot.age index builds");
        // No Alien index — confirms expansion still fires when at least
        // one branch has an applicable index.
        let catalog = MockIndexCatalog::new()
            .with_node_typed_index(person.clone(), email.clone(), selene_gql::IndexKind::String)
            .with_node_typed_index(robot.clone(), email, selene_gql::IndexKind::String)
            .with_node_typed_index(person, age.clone(), selene_gql::IndexKind::Integer)
            .with_node_typed_index(robot, age, selene_gql::IndexKind::Integer);
        Self { graph, catalog }
    }
}

// ---------------------------------------------------------------------------
// Row-set equivalence with the manual UNION ALL rewrite
// ---------------------------------------------------------------------------

#[test]
fn row_set_equivalence_with_manual_union_all() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);

    let expanded_plan = optimized(
        "MATCH (n:Person|Robot|Alien) WHERE n.email = 'alice' RETURN n",
        &fixture.catalog,
    );
    let manual_plan = optimized(
        "MATCH (n:Person) WHERE n.email = 'alice' RETURN n \
         UNION ALL \
         MATCH (n:Robot) WHERE n.email = 'alice' RETURN n \
         UNION ALL \
         MATCH (n:Alien) WHERE n.email = 'alice' RETURN n",
        &fixture.catalog,
    );

    let expanded =
        rows(execute_optimized(&mut session, &expanded_plan).expect("expanded executes"));
    let manual = rows(execute_optimized(&mut session, &manual_plan).expect("manual executes"));

    let mut expanded_ids = node_ref_ids(&expanded);
    let mut manual_ids = node_ref_ids(&manual);
    expanded_ids.sort_unstable();
    manual_ids.sort_unstable();
    assert_eq!(
        expanded_ids, manual_ids,
        "expanded form must produce the same node-id set as the manual UNION ALL"
    );
    assert_eq!(
        expanded_ids.len(),
        1,
        "exactly one node has email == 'alice' (the Person)"
    );
}

// ---------------------------------------------------------------------------
// Composition with BRIEF-154 parameter-aware index selection
// ---------------------------------------------------------------------------

#[test]
fn composition_with_parameterized_index() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);
    session.bind_parameter(istr("target"), Value::String(istr("r2d2")));

    let plan = optimized(
        "MATCH (n:Person|Robot) WHERE n.email = $target RETURN n",
        &fixture.catalog,
    );
    let table = rows(execute_optimized(&mut session, &plan).expect("parameterized executes"));

    let ids = node_ref_ids(&table);
    assert_eq!(ids.len(), 1, "exactly one Robot has email == 'r2d2'");
}

#[test]
fn composition_with_parameterized_typed_range() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);
    session.bind_parameter(istr("min_age"), Value::Int(50));

    let plan = optimized(
        "MATCH (n:Person|Robot) WHERE n.age >= $min_age RETURN n",
        &fixture.catalog,
    );
    let table = rows(execute_optimized(&mut session, &plan).expect("parameterized executes"));

    // 0 Persons with age >= 50 (20, 25, 30) + 2 Robots (100, 125) = 2 rows.
    assert_eq!(node_ref_ids(&table).len(), 2);
}

// ---------------------------------------------------------------------------
// Composition with BRIEF-153 ExternalString carve-out
// ---------------------------------------------------------------------------

#[test]
fn composition_with_external_string_carve_out() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);
    session.bind_parameter(istr("target"), Value::ExternalString(Arc::from("alice")));

    let plan = optimized(
        "MATCH (n:Person|Robot) WHERE n.email = $target RETURN n",
        &fixture.catalog,
    );
    let table =
        rows(execute_optimized(&mut session, &plan).expect("ExternalString param executes"));

    let ids = node_ref_ids(&table);
    assert_eq!(
        ids.len(),
        1,
        "exactly one Person has email == 'alice' (ExternalString equivalence)"
    );
}

// ---------------------------------------------------------------------------
// Composition with downstream pipeline ops
// ---------------------------------------------------------------------------

#[test]
fn composition_with_downstream_limit() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);

    // 3 Persons + 2 Robots + 1 Alien = 6 candidate rows. LIMIT 3 applied
    // at the pipeline level (not per branch) returns exactly 3 — pins
    // that option (b) IR wraps the union at JoinTree level so LIMIT
    // operates on the unioned binding table, not the per-branch fan-out.
    let plan = optimized(
        "MATCH (n:Person|Robot|Alien) WHERE n.email = 'alice' OR n.email = 'r2d2' OR n.email = 'zorblax' RETURN n LIMIT 3",
        &fixture.catalog,
    );
    let table = rows(execute_optimized(&mut session, &plan).expect("LIMIT executes"));
    assert_eq!(table.rows().len(), 3);
}

#[test]
fn composition_with_downstream_order_by() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);

    let plan = optimized(
        "MATCH (n:Person|Robot) WHERE n.age <= 200 RETURN n.age AS age ORDER BY n.age",
        &fixture.catalog,
    );
    let table = rows(execute_optimized(&mut session, &plan).expect("ORDER BY executes"));
    let ages: Vec<i64> = table
        .rows()
        .iter()
        .filter_map(|row| match row.values().first() {
            Some(Value::Int(age)) => Some(*age),
            _ => None,
        })
        .collect();
    // Persons: 20, 25, 30; Robots: 100, 125. All ≤ 200, sorted ascending.
    assert_eq!(ages, vec![20, 25, 30, 100, 125]);
}

#[test]
fn composition_with_downstream_group_by() {
    let fixture = LabelFamilyFixture::build();
    let mut session = Session::new(&fixture.graph);

    let plan = optimized(
        "MATCH (n:Person|Robot) WHERE n.age <= 200 RETURN n.department AS dept, count(*) AS c GROUP BY n.department",
        &fixture.catalog,
    );
    let table = rows(execute_optimized(&mut session, &plan).expect("GROUP BY executes"));

    // Single group ("engineering" = 5).
    assert_eq!(table.rows().len(), 1);
    let dept_index = table
        .schema()
        .columns
        .iter()
        .position(|column| {
            column
                .name
                .clone()
                .is_some_and(|name| name.as_str() == "dept")
        })
        .expect("dept column");
    let count_index = table
        .schema()
        .columns
        .iter()
        .position(|column| column.name.clone().is_some_and(|name| name.as_str() == "c"))
        .expect("c column");
    let row = &table.rows()[0];
    assert_eq!(
        row.get(dept_index).cloned().unwrap_or(Value::Null),
        Value::String(istr("engineering"))
    );
    assert_eq!(
        row.get(count_index).cloned().unwrap_or(Value::Null),
        Value::Int(5)
    );
}

// ---------------------------------------------------------------------------
// Q11 / PR #177 C1 — multi-label nodes dedup at the JoinTree level so the
// rule-firing path matches the unexpanded `LabelExpr::Disjunction(any)`
// semantics — same query + same data => same rows out, regardless of
// whether the optimizer rule fired.
// ---------------------------------------------------------------------------

#[test]
fn multi_label_node_dedups_at_disjunctive_scan_join_tree_level() {
    // A node carrying labels A AND B would otherwise appear once per
    // branch (the unfixed BRIEF-155 shipped that as Q11 "UNION ALL"
    // semantics). PR #177 Codex C1 caught that this changes COUNT /
    // LIMIT / aggregates observably based on whether the
    // disjunctive-label-expansion rule fired (catalog-present) vs the
    // unexpanded baseline (catalog-absent). The fix dedups at the
    // `JoinTree::DisjunctiveScan` executor arm so the union-then-dedup'd
    // binding table matches the unexpanded
    // `LabelExpr::Disjunction(any(...))` semantics, which visit each node
    // once.
    let person = istr("Multi1Person");
    let robot = istr("Multi1Robot");
    let email = istr("email");

    let graph = SharedGraph::new(GraphId::new(1551));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::from_iter([person.clone(), robot.clone()]),
                props([(email.clone(), Value::String(istr("hybrid")))]),
            )
            .expect("multi-label node inserts");
        mutator
            .create_node(
                LabelSet::single(person.clone()),
                props([(email.clone(), Value::String(istr("hybrid")))]),
            )
            .expect("Person-only node inserts");
        txn.commit().expect("fixture commits");
    }
    graph
        .create_property_index(person.clone(), email.clone(), TypedIndexKind::String)
        .expect("Person.email index builds");
    graph
        .create_property_index(robot.clone(), email.clone(), TypedIndexKind::String)
        .expect("Robot.email index builds");
    let catalog = MockIndexCatalog::new()
        .with_node_typed_index(person, email.clone(), selene_gql::IndexKind::String)
        .with_node_typed_index(robot, email, selene_gql::IndexKind::String);

    let plan = optimized(
        "MATCH (n:Multi1Person|Multi1Robot) WHERE n.email = 'hybrid' RETURN n",
        &catalog,
    );
    let mut session = Session::new(&graph);
    let table = rows(execute_optimized(&mut session, &plan).expect("multi-label executes"));

    // 2 rows post-dedup: the multi-label `hybrid` node appears ONCE
    // (deduped across the Person and Robot branches), plus the
    // Person-only `hybrid` node.
    assert_eq!(table.rows().len(), 2);

    let ids = node_ref_ids(&table);
    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    for id in ids {
        *counts.entry(id).or_default() += 1;
    }
    assert!(
        counts.values().all(|count| *count == 1),
        "every node id must appear exactly once after JoinTree-level dedup; got counts {counts:?}"
    );
    assert_eq!(
        counts.len(),
        2,
        "two distinct nodes match (multi-label hybrid + Person-only hybrid)"
    );
}

#[test]
fn catalog_present_vs_absent_produces_identical_row_set() {
    // PR #177 Codex C1 invariant: same query + same data over two
    // catalogs (one whose per-label typed indexes make the
    // disjunctive-label-expansion rule fire, one without any indexes
    // that leaves the scan as the unexpanded
    // `LabelExpr::Disjunction(any)` baseline) MUST yield identical row
    // sets. Without the JoinTree-level dedup at
    // `JoinTree::DisjunctiveScan`, a multi-label node appears once per
    // matching branch in the expanded plan and exactly once in the
    // unexpanded baseline — directly observable via COUNT / LIMIT /
    // aggregates.
    let person = istr("InvPerson");
    let robot = istr("InvRobot");
    let email = istr("email");

    let graph = SharedGraph::new(GraphId::new(1552));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        // Two multi-label nodes (both Person+Robot) + one Person-only +
        // one Robot-only. Multi-label nodes are the dedup-sensitive
        // case — without dedup the expanded plan double-counts them.
        for tag in ["alpha", "beta"] {
            mutator
                .create_node(
                    LabelSet::from_iter([person.clone(), robot.clone()]),
                    props([(email.clone(), Value::String(istr(tag)))]),
                )
                .expect("multi-label node inserts");
        }
        mutator
            .create_node(
                LabelSet::single(person.clone()),
                props([(email.clone(), Value::String(istr("gamma")))]),
            )
            .expect("Person-only node inserts");
        mutator
            .create_node(
                LabelSet::single(robot.clone()),
                props([(email.clone(), Value::String(istr("delta")))]),
            )
            .expect("Robot-only node inserts");
        txn.commit().expect("fixture commits");
    }
    // Storage indexes are needed for the rule's downstream
    // `composite_index_lookup` / `range_index_scan` / `in_list_optimization`
    // passes (slots 6/7/8) to materialize the index access at runtime.
    // They live on the graph regardless of which catalog the optimizer
    // sees, so they are not load-bearing for the catalog-present vs
    // catalog-absent comparison — the comparison is driven by the
    // optimizer-time `MockIndexCatalog`.
    graph
        .create_property_index(person.clone(), email.clone(), TypedIndexKind::String)
        .expect("Person.email index builds");
    graph
        .create_property_index(robot.clone(), email.clone(), TypedIndexKind::String)
        .expect("Robot.email index builds");

    // Catalog WITH per-label indexes — rule fires, plan becomes
    // `JoinTree::DisjunctiveScan { branches: [Person, Robot] }`.
    let catalog_with_indexes = MockIndexCatalog::new()
        .with_node_typed_index(person, email.clone(), selene_gql::IndexKind::String)
        .with_node_typed_index(robot, email, selene_gql::IndexKind::String);
    // Catalog WITHOUT — rule's `any_branch_has_applicable_index` gate
    // returns false, plan stays as the unexpanded `JoinTree::Scan` with
    // `LabelExpr::Disjunction([Person, Robot])`.
    let catalog_without_indexes = MockIndexCatalog::new();

    let query = "MATCH (n:InvPerson|InvRobot) RETURN n";
    let plan_with = optimized(query, &catalog_with_indexes);
    let plan_without = optimized(query, &catalog_without_indexes);

    let mut session = Session::new(&graph);
    let with_rows =
        rows(execute_optimized(&mut session, &plan_with).expect("with-catalog executes"));
    let without_rows =
        rows(execute_optimized(&mut session, &plan_without).expect("without-catalog executes"));

    let mut with_ids = node_ref_ids(&with_rows);
    let mut without_ids = node_ref_ids(&without_rows);
    with_ids.sort_unstable();
    without_ids.sort_unstable();
    assert_eq!(
        with_ids, without_ids,
        "catalog-present vs catalog-absent plans must yield identical row sets — \
         the JoinTree-level dedup at DisjunctiveScan preserves query semantics across \
         optimizer rule firing"
    );
    // Sanity: 4 distinct nodes total (alpha + beta multi-label, gamma
    // Person-only, delta Robot-only). Without the dedup fix, the
    // catalog-present plan would emit 6 rows (alpha+beta counted twice
    // each via Person and Robot branches).
    assert_eq!(
        with_ids.len(),
        4,
        "exactly 4 distinct nodes match `(n:InvPerson|InvRobot)`"
    );
}
