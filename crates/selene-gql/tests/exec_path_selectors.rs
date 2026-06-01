//! Executor path-selector tests.

mod exec_common;

use exec_common::{istr, node_ids_for, planned, props};
use selene_core::{CancellationToken, GraphId, LabelSet, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutorError, TxContext,
    execute_pattern, execute_pipeline,
};
use selene_graph::SharedGraph;

struct PathSelectorFixture {
    graph: SharedGraph,
}

impl PathSelectorFixture {
    fn build() -> Self {
        let node = istr("N");
        let edge = istr("K");
        let name = istr("name");
        let graph = SharedGraph::new(GraphId::new(6332));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("A")))]),
                )
                .expect("A inserts");
            let b1 = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("B1")))]),
                )
                .expect("B1 inserts");
            let b2 = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("B2")))]),
                )
                .expect("B2 inserts");
            let c = mutator
                .create_node(
                    LabelSet::single(node.clone()),
                    props([(name.clone(), Value::String(istr("C")))]),
                )
                .expect("C inserts");
            let d = mutator
                .create_node(
                    LabelSet::single(node),
                    props([(name, Value::String(istr("D")))]),
                )
                .expect("D inserts");

            mutator
                .create_edge(edge.clone(), a, c, props([]))
                .expect("edge 1");
            mutator
                .create_edge(edge.clone(), a, c, props([]))
                .expect("edge 2");
            mutator
                .create_edge(edge.clone(), a, b1, props([]))
                .expect("edge 3");
            mutator
                .create_edge(edge.clone(), b1, c, props([]))
                .expect("edge 4");
            mutator
                .create_edge(edge.clone(), a, b2, props([]))
                .expect("edge 5");
            mutator
                .create_edge(edge.clone(), b2, c, props([]))
                .expect("edge 6");
            mutator
                .create_edge(edge.clone(), b1, d, props([]))
                .expect("edge 7");
            mutator.create_edge(edge, d, c, props([])).expect("edge 8");
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
fn all_selector_preserves_rows_across_path_shapes() {
    let fixture = PathSelectorFixture::build();

    assert_eq!(
        fixture.row_count("MATCH ALL (a:N {name: 'A'})-[:K]->(c:N {name: 'C'}) RETURN c"),
        2
    );
    assert_eq!(
        fixture.row_count("MATCH ALL (a:N {name: 'A'})-[:K*1..3]->(c:N {name: 'C'}) RETURN c"),
        5
    );
    assert_eq!(
        fixture.row_count("MATCH ALL (a:N {name: 'A'})-[:K]->(m)-[:K]->(c:N {name: 'C'}) RETURN m"),
        2
    );
    assert_eq!(
        fixture.row_count(
            "MATCH ALL (a:N {name: 'A'})-[:K]->(m)-[r:K*1..2]->(c:N {name: 'C'}) RETURN r"
        ),
        3
    );
}

#[test]
fn any_selector_keeps_one_row_per_endpoint_pair_across_path_shapes() {
    let fixture = PathSelectorFixture::build();

    assert_eq!(
        fixture.row_count("MATCH ANY (a:N {name: 'A'})-[:K]->(c:N {name: 'C'}) RETURN c"),
        1
    );
    assert_eq!(
        fixture.row_count("MATCH ANY (a:N {name: 'A'})-[:K*1..3]->(c:N {name: 'C'}) RETURN c"),
        1
    );
    assert_eq!(
        fixture.row_count("MATCH ANY (a:N {name: 'A'})-[:K]->(m)-[:K]->(c:N {name: 'C'}) RETURN m"),
        1
    );
    assert_eq!(
        fixture.row_count(
            "MATCH ANY (a:N {name: 'A'})-[:K]->(m)-[r:K*1..2]->(c:N {name: 'C'}) RETURN r"
        ),
        1
    );
}

#[test]
fn all_shortest_keeps_all_minimum_hop_rows_across_path_shapes() {
    let fixture = PathSelectorFixture::build();

    assert_eq!(
        fixture.row_count("MATCH ALL SHORTEST (a:N {name: 'A'})-[:K]->(c:N {name: 'C'}) RETURN c"),
        2
    );
    let quantified = fixture
        .execute("MATCH ALL SHORTEST (a:N {name: 'A'})-[r:K*1..3]->(c:N {name: 'C'}) RETURN r")
        .expect("query executes");
    assert_eq!(edge_list_lengths(&quantified, "r"), vec![1, 1]);
    assert_eq!(
        fixture.row_count(
            "MATCH ALL SHORTEST (a:N {name: 'A'})-[:K]->(m)-[:K]->(c:N {name: 'C'}) RETURN m"
        ),
        2
    );
    let mixed = fixture
        .execute(
            "MATCH ALL SHORTEST (a:N {name: 'A'})-[:K]->(m)-[r:K*1..2]->(c:N {name: 'C'}) RETURN r",
        )
        .expect("query executes");
    assert_eq!(edge_list_lengths(&mixed, "r"), vec![1, 1]);
}

#[test]
fn any_shortest_keeps_one_minimum_hop_row_across_path_shapes() {
    let fixture = PathSelectorFixture::build();

    assert_eq!(
        fixture.row_count("MATCH ANY SHORTEST (a:N {name: 'A'})-[:K]->(c:N {name: 'C'}) RETURN c"),
        1
    );
    assert_eq!(
        fixture.row_count(
            "MATCH ANY SHORTEST (a:N {name: 'A'})-[:K*1..3]->(c:N {name: 'C'}) RETURN c"
        ),
        1
    );
    assert_eq!(
        fixture.row_count(
            "MATCH ANY SHORTEST (a:N {name: 'A'})-[:K]->(m)-[:K]->(c:N {name: 'C'}) RETURN m"
        ),
        1
    );
    assert_eq!(
        fixture.row_count(
            "MATCH ANY SHORTEST (a:N {name: 'A'})-[:K]->(m)-[r:K*1..2]->(c:N {name: 'C'}) RETURN r"
        ),
        1
    );
}

#[test]
fn pure_node_selector_is_zero_hop_noop() {
    let fixture = PathSelectorFixture::build();
    for selector in ["ALL", "ANY", "ALL SHORTEST", "ANY SHORTEST"] {
        let source = format!("MATCH {selector} (a:N {{name: 'A'}}) RETURN a");
        assert_eq!(fixture.row_count(&source), 1);
    }
}

#[test]
fn anonymous_quantified_edge_under_selector_uses_hidden_hop_list() {
    let fixture = PathSelectorFixture::build();

    let table = fixture
        .execute("MATCH ANY SHORTEST (a:N {name: 'A'})-[:K*1..3]->(c:N {name: 'C'}) RETURN c")
        .expect("query executes");

    assert_eq!(node_ids_for(&table, "c"), vec![Some(4)]);
}

#[test]
fn path_selector_composes_under_optional_match() {
    let fixture = PathSelectorFixture::build();

    let table = fixture
        .execute(
            "MATCH (a:N {name: 'A'}) OPTIONAL MATCH ANY SHORTEST (a)-[:K*1..2]->(z:N {name: 'Z'}) RETURN a, z",
        )
        .expect("query executes");

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1)]);
    assert_eq!(node_ids_for(&table, "z"), vec![None]);
}

#[test]
fn path_selector_keeps_repeat_inline_group_predicate_shape() {
    let fixture = PathSelectorFixture::build();

    let table = fixture
        .execute(
            "MATCH ALL SHORTEST (a:N {name: 'A'})-[r:K*1..3 WHERE size(r) = 1]->(c:N {name: 'C'}) RETURN r",
        )
        .expect("query executes");

    assert_eq!(edge_list_lengths(&table, "r"), vec![1, 1]);
}

#[test]
fn shortest_selectors_default_to_trail() {
    let node = istr("N");
    let edge = istr("K");
    let name = istr("name");
    let graph = SharedGraph::new(GraphId::new(6334));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(
                LabelSet::single(node),
                props([(name, Value::String(istr("A")))]),
            )
            .expect("A inserts");
        mutator
            .create_edge(edge, a, a, props([]))
            .expect("loop edge");
        txn.commit().expect("fixture commits");
    }
    let fixture = PathSelectorFixture { graph };

    assert_eq!(
        fixture.row_count(
            "MATCH ALL SHORTEST (a:N {name: 'A'})-[:K*2..2]->(b:N {name: 'A'}) RETURN b"
        ),
        0
    );
    assert_eq!(
        fixture.row_count(
            "MATCH ANY SHORTEST (a:N {name: 'A'})-[:K]->(:N {name: 'A'})-[:K]->(b:N {name: 'A'}) RETURN b"
        ),
        0
    );
}

#[test]
fn path_selector_checks_cancellation_while_filtering_rows() {
    let root = istr("Root");
    let graph = SharedGraph::new(GraphId::new(6333));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for _ in 0..1100 {
            mutator
                .create_node(LabelSet::single(root.clone()), props([]))
                .expect("root inserts");
        }
        txn.commit().expect("fixture commits");
    }

    let plan = planned("MATCH ANY (n:Root) RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let token = CancellationToken::new();
    token.cancel();
    let ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries)
    .with_resource_limits(Some(&token), None, None);

    let err = execute_pattern(pattern, &ctx).expect_err("path selector observes token");

    assert!(matches!(err, ExecutorError::Cancelled { .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL2");
}
