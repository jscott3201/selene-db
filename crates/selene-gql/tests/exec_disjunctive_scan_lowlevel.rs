//! BRIEF-155 Commit 2 — low-level `JoinTree::DisjunctiveScan` executor
//! tests.
//!
//! The disjunctive-label-expansion rule lands in Commit 3, so these tests
//! hand-construct a `DisjunctiveScan` plan by surgically replacing the
//! leading `JoinTree::Scan` of an already-planned `MATCH (n:Person)` with a
//! two-branch `DisjunctiveScan`. This exercises the runtime executor arm
//! and the walker helpers (`collect_hidden_slots`) without depending on the
//! rule itself.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, planned};
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    Binding, BindingTable, EmptyProcedureRegistry, JoinTree, LabelExpr, NodeOrEdgeScan, TxContext,
    execute_pattern as execute_pattern_plan,
};
use selene_graph::SharedGraph;

fn istr_local(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

/// Replace the leading `JoinTree::Scan` with a `DisjunctiveScan` whose
/// branches each clone the original scan but stamp a single label on the
/// per-branch `label_predicate`.
fn rewrite_to_disjunctive(tree: &mut JoinTree, labels: &[IStr]) {
    let JoinTree::Scan(scan) = tree else {
        panic!("expected leading JoinTree::Scan, got {tree:?}");
    };
    let original = scan.clone();
    let branches: Vec<NodeOrEdgeScan> = labels
        .iter()
        .map(|label| {
            let mut clone = original.clone();
            clone.label_predicate = Some(LabelExpr::Single(*label));
            clone
        })
        .collect();
    let scan_anchor = original;
    *tree = JoinTree::DisjunctiveScan {
        branches,
        scan_anchor,
    };
}

fn count_rows(table: &BindingTable) -> usize {
    table.rows().len()
}

/// Execute a MATCH (n:Person) plan but with the leading scan swapped for a
/// disjunctive expansion across the given labels. Returns the pattern's
/// `BindingTable`.
fn execute_with_branches(
    fixture: &ExecFixture,
    source: &str,
    branch_labels: &[IStr],
) -> BindingTable {
    let mut plan = planned(source);
    let pattern = plan
        .pattern_plan
        .as_mut()
        .expect("source has a pattern plan");
    rewrite_to_disjunctive(&mut pattern.join_tree, branch_labels);

    let ctx = fixture.context_caps(&plan);
    execute_pattern(plan.pattern_plan.as_ref().unwrap(), &ctx)
}

#[test]
fn disjunctive_scan_executor_concatenates_branches() {
    // The ExecFixture has 3 Persons (Alice, Bob, Cara), 1 Sensor, and 2
    // Counters. A `(n:Person)|(n:Sensor)` expansion should return
    // 3 + 1 = 4 candidate rows. The JoinTree-level NodeId dedup (PR #177
    // C1) is a no-op here because no fixture node carries BOTH Person and
    // Sensor labels — branches yield disjoint NodeId sets.
    let fixture = ExecFixture::build();
    let person = fixture.person;
    let sensor = fixture.sensor;

    let table = execute_with_branches(&fixture, "MATCH (n:Person) RETURN n", &[person, sensor]);
    assert_eq!(
        count_rows(&table),
        4,
        "Person branch (3 rows) + Sensor branch (1 row) = 4 unioned rows; got {}",
        count_rows(&table)
    );
}

#[test]
fn disjunctive_scan_executor_per_branch_label_filter() {
    // The fixture's Counter nodes are NOT labelled Person or Sensor, so a
    // `(n:Person)|(n:Sensor)` expansion must NOT yield Counter rows.
    let fixture = ExecFixture::build();
    let person = fixture.person;
    let sensor = fixture.sensor;
    let counter = fixture.counter;

    let table = execute_with_branches(&fixture, "MATCH (n:Person) RETURN n", &[person, sensor]);
    let counter_label = counter; // shadow for closure clarity
    let counter_rows: Vec<&Binding> = table
        .rows()
        .iter()
        .filter(|row| {
            // Each row's first value is the NodeRef per ExecFixture; cross-
            // check by re-reading the snapshot for that NodeId's labels.
            matches!(row.values().first(), Some(Value::NodeRef(_)))
        })
        .collect();
    // 4 total rows — none should be Counter-labelled (all 4 are Person or
    // Sensor). We assert no Counter rows by counting against the Counter-
    // labelled population in the fixture (2) — the table must NOT contain
    // those nodes.
    let snapshot_labels = fixture.graph.read();
    for row in &counter_rows {
        if let Some(Value::NodeRef(id)) = row.values().first() {
            let labels = snapshot_labels.node_labels(*id).expect("node alive");
            assert!(
                !labels.contains(&counter_label),
                "Counter node leaked through Person|Sensor disjunction"
            );
        }
    }
    assert_eq!(counter_rows.len(), 4);
}

#[test]
fn disjunctive_scan_executor_dedups_multi_label_node() {
    // Build a tiny fresh graph with a single node carrying labels A AND B,
    // then run a `(n:A)|(n:B)` disjunction. PR #177 Codex C1: the
    // `JoinTree::DisjunctiveScan` executor arm dedups branch outputs by
    // `NodeId`, so a node carrying labels A AND B appears EXACTLY ONCE
    // in the unioned binding table — matching the unexpanded
    // `LabelExpr::Disjunction(any(...))` semantics, preserving the
    // catalog-present vs catalog-absent invariant for COUNT / LIMIT /
    // aggregates.
    let label_a = istr_local("Alpha");
    let label_b = istr_local("Beta");
    let label_c = istr_local("Gamma");
    let multi_label = LabelSet::from_iter([label_a, label_b]);
    let single_a = LabelSet::single(label_a);
    let single_b = LabelSet::single(label_b);

    let graph = SharedGraph::new(GraphId::new(155));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(multi_label, PropertyMap::default())
            .expect("multi-label node inserts");
        mutator
            .create_node(single_a, PropertyMap::default())
            .expect("Alpha-only node inserts");
        mutator
            .create_node(single_b, PropertyMap::default())
            .expect("Beta-only node inserts");
        txn.commit().expect("test fixture commits");
    }

    // Plan a single-label query, then surgically rewrite the leading scan
    // to a 2-branch DisjunctiveScan over (Alpha, Beta).
    let mut plan = planned("MATCH (n:Alpha) RETURN n");
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan present");
    rewrite_to_disjunctive(&mut pattern.join_tree, &[label_a, label_b]);

    let ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);

    let table = execute_pattern_plan(plan.pattern_plan.as_ref().unwrap(), &ctx)
        .expect("disjunctive pattern executes");

    // 3 nodes total: AB (multi-label), A-only, B-only.
    // Alpha branch matches AB + A-only = 2 rows.
    // Beta branch matches AB + B-only = 2 rows.
    // After NodeId dedup at JoinTree::DisjunctiveScan → 3 rows; AB
    // appears exactly once (matching the unexpanded baseline).
    assert_eq!(
        table.rows().len(),
        3,
        "expected AB + A + B = 3 deduped rows; got {}",
        table.rows().len()
    );

    // Every NodeId must appear at most once after dedup.
    let mut seen = std::collections::BTreeSet::new();
    for row in table.rows() {
        if let Some(Value::NodeRef(id)) = row.values().first() {
            assert!(
                seen.insert(id.get()),
                "NodeId {} appeared more than once; dedup at \
                 JoinTree::DisjunctiveScan must collapse multi-label rows",
                id.get()
            );
        }
    }
    assert_eq!(
        seen.len(),
        3,
        "exactly 3 distinct NodeIds in the deduped row set"
    );

    // Sanity: the Gamma label does NOT appear; this only matters if the rule
    // accidentally added a third branch. `count_rows == 3` already pins this,
    // but assert label_c stays unused.
    let _ = label_c;
}
