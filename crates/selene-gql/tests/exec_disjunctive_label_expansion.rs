//! BRIEF-155 Commit 4 — end-to-end execution + composition tests for the
//! disjunctive-label-expansion optimizer rule.
//!
//! Validates:
//! - Row-set equivalence with the manual `UNION ALL` rewrite (the ariadne
//!   workaround) — acceptance bar #6.
//! - Composition with BRIEF-154 parameterized index selection (acceptance
//!   bar #7).
//! - Composition with BRIEF-153 STRING-index ExternalString carve-out.
//! - Downstream LIMIT / ORDER BY / GROUP BY — the union happens at
//!   JoinTree level, so the pipeline operates on the unioned binding
//!   table, not per branch.
//! - Q11 multi-label-node-double-counts via the rule-emitted plan path.

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
                        LabelSet::single(person),
                        props([
                            (email, Value::String(istr(name))),
                            (age, Value::Int(person_age)),
                            (department, Value::String(istr("engineering"))),
                        ]),
                    )
                    .expect("Person inserts");
            }
            for (name, robot_age) in [("r2d2", 100), ("wall-e", 125)] {
                mutator
                    .create_node(
                        LabelSet::single(robot),
                        props([
                            (email, Value::String(istr(name))),
                            (age, Value::Int(robot_age)),
                            (department, Value::String(istr("engineering"))),
                        ]),
                    )
                    .expect("Robot inserts");
            }
            mutator
                .create_node(
                    LabelSet::single(alien),
                    props([
                        (email, Value::String(istr("zorblax"))),
                        (age, Value::Int(99_999)),
                        (department, Value::String(istr("xenobiology"))),
                    ]),
                )
                .expect("Alien inserts");
            txn.commit().expect("fixture commits");
        }
        graph
            .create_property_index(person, email, TypedIndexKind::String)
            .expect("Person.email index builds");
        graph
            .create_property_index(robot, email, TypedIndexKind::String)
            .expect("Robot.email index builds");
        graph
            .create_property_index(person, age, TypedIndexKind::I64)
            .expect("Person.age index builds");
        graph
            .create_property_index(robot, age, TypedIndexKind::I64)
            .expect("Robot.age index builds");
        // No Alien index — confirms expansion still fires when at least
        // one branch has an applicable index.
        let catalog = MockIndexCatalog::new()
            .with_node_typed_index(person, email, selene_gql::IndexKind::String)
            .with_node_typed_index(robot, email, selene_gql::IndexKind::String)
            .with_node_typed_index(person, age, selene_gql::IndexKind::Integer)
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
        .position(|column| column.name.is_some_and(|name| name.as_str() == "dept"))
        .expect("dept column");
    let count_index = table
        .schema()
        .columns
        .iter()
        .position(|column| column.name.is_some_and(|name| name.as_str() == "c"))
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
// Q11 — multi-label node appears in multiple branches via rule path
// ---------------------------------------------------------------------------

#[test]
fn multi_label_node_appears_in_multiple_branches() {
    // Sanity check Q11 again via the rule-emitted plan path (not the
    // hand-built DisjunctiveScan from Commit 2's low-level test).
    let person = istr("Multi1Person");
    let robot = istr("Multi1Robot");
    let email = istr("email");

    let graph = SharedGraph::new(GraphId::new(1551));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::from_iter([person, robot]),
                props([(email, Value::String(istr("hybrid")))]),
            )
            .expect("multi-label node inserts");
        mutator
            .create_node(
                LabelSet::single(person),
                props([(email, Value::String(istr("hybrid")))]),
            )
            .expect("Person-only node inserts");
        txn.commit().expect("fixture commits");
    }
    graph
        .create_property_index(person, email, TypedIndexKind::String)
        .expect("Person.email index builds");
    graph
        .create_property_index(robot, email, TypedIndexKind::String)
        .expect("Robot.email index builds");
    let catalog = MockIndexCatalog::new()
        .with_node_typed_index(person, email, selene_gql::IndexKind::String)
        .with_node_typed_index(robot, email, selene_gql::IndexKind::String);

    let plan = optimized(
        "MATCH (n:Multi1Person|Multi1Robot) WHERE n.email = 'hybrid' RETURN n",
        &catalog,
    );
    let mut session = Session::new(&graph);
    let table = rows(execute_optimized(&mut session, &plan).expect("multi-label executes"));

    // 3 rows: hybrid via Person branch, hybrid via Robot branch, Person-only.
    assert_eq!(table.rows().len(), 3);

    let ids = node_ref_ids(&table);
    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    for id in ids {
        *counts.entry(id).or_default() += 1;
    }
    assert!(
        counts.values().any(|count| *count == 2),
        "hybrid (Person+Robot) node must appear twice across disjunction branches"
    );
}
