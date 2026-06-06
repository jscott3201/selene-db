//! Executor path-mode tests.

mod exec_common;

use exec_common::{db_string, planned, props};
use selene_core::{GraphId, LabelSet, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutorError, TxContext,
    execute_pattern, execute_pipeline,
};
use selene_graph::SharedGraph;

struct PathModeFixture {
    graph: SharedGraph,
}

impl PathModeFixture {
    fn build() -> Self {
        let node = db_string("N");
        let edge = db_string("K");
        let name = db_string("name");
        let graph = SharedGraph::new(GraphId::new(6335));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(db_string("A")))]),
                )
                .expect("A inserts");
            let b = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(db_string("B")))]),
                )
                .expect("B inserts");
            let c = mutator
                .create_node(
                    LabelSet::single(node),
                    props([(name, Value::String(db_string("C")))]),
                )
                .expect("C inserts");
            mutator
                .create_edge(edge.clone(), a, a, props([]))
                .expect("A loop");
            mutator
                .create_edge(edge.clone(), a, b, props([]))
                .expect("A to B");
            mutator
                .create_edge(edge.clone(), b, a, props([]))
                .expect("B to A");
            mutator.create_edge(edge, a, c, props([])).expect("A to C");
            txn.commit().expect("fixture commits");
        }
        Self { graph }
    }

    fn execute(&self, source: &str) -> Result<BindingTable, ExecutorError> {
        let plan = planned(source);
        let mut ctx = TxContext::read_only(
            self.graph.read(),
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            self.graph.index_providers(),
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

    fn row_count(&self, source: &str) -> usize {
        self.execute(source).expect("query executes").row_count()
    }
}

fn edge_list_lengths(table: &BindingTable, name: &str) -> Vec<usize> {
    exec_common::column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::List(items) => items.len(),
            other => panic!("expected edge list, got {other:?}"),
        })
        .collect()
}

#[test]
fn trail_rejects_repeated_edge_from_quantified_self_loop() {
    let fixture = PathModeFixture::build();

    assert_eq!(
        fixture.row_count("MATCH TRAIL (a:N {name: 'A'})-[:K*2..2]->(b:N {name: 'A'}) RETURN b"),
        1
    );
    assert_eq!(
        fixture.row_count("MATCH TRAIL (a:N {name: 'A'})-[r:K*2..2]->(b:N {name: 'A'}) RETURN r"),
        1
    );
    assert_eq!(
        fixture.row_count("MATCH WALK (a:N {name: 'A'})-[:K*2..2]->(b:N {name: 'A'}) RETURN b"),
        2
    );
}

#[test]
fn simple_accepts_closed_walk_that_acyclic_rejects() {
    let fixture = PathModeFixture::build();

    assert_eq!(
        fixture.row_count("MATCH SIMPLE (a:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"),
        1
    );
    assert_eq!(
        fixture.row_count("MATCH ACYCLIC (a:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"),
        0
    );
}

#[test]
fn trail_filters_fixed_chain_reusing_same_edge() {
    let fixture = PathModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH TRAIL (a:N {name: 'A'})-[:K]->(m:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        0
    );
    assert_eq!(
        fixture.row_count(
            "MATCH WALK (a:N {name: 'A'})-[:K]->(m:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        1
    );
}

#[test]
fn acyclic_filters_mixed_expand_repeat_revisit() {
    let fixture = PathModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH ACYCLIC (a:N {name: 'A'})-[:K]->(m:N {name: 'B'})-[r:K*1..1]->(b:N {name: 'A'}) RETURN r"
        ),
        0
    );
    assert_eq!(
        fixture.row_count(
            "MATCH SIMPLE (a:N {name: 'A'})-[:K]->(m:N {name: 'B'})-[r:K*1..1]->(b:N {name: 'A'}) RETURN r"
        ),
        1
    );
}

#[test]
fn shortest_selector_filters_to_trail_valid_paths_first() {
    let fixture = PathModeFixture::build();

    assert_eq!(
        fixture.row_count(
            "MATCH ALL SHORTEST TRAIL (a:N {name: 'A'})-[:K*2..2]->(b:N {name: 'A'}) RETURN b"
        ),
        1
    );
    assert_eq!(
        fixture.row_count("MATCH ALL SHORTEST ACYCLIC (a:N {name: 'A'})-[:K*1..1]->(b) RETURN b"),
        2
    );
}

#[test]
fn zero_hop_simple_accepts_empty_path() {
    let fixture = PathModeFixture::build();

    let table = fixture
        .execute("MATCH SIMPLE (a:N {name: 'A'})-[r:K*0..0]->(b:N {name: 'A'}) RETURN r")
        .expect("query executes");

    assert_eq!(edge_list_lengths(&table, "r"), vec![0]);
}
