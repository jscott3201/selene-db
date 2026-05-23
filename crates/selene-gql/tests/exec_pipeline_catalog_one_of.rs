//! BRIEF-131e commit-2 acceptance tests: resolver cascade + SHOW EDGE TYPES
//! round-trip for `EdgeEndpointDef::OneOf`.

mod exec_common;

use selene_core::{GraphId, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, PipelineOp, TxContext, analyze, execute_pipeline, parse, plan,
};
use selene_graph::{EdgeEndpointDef, GraphError, GraphTypeDef, SharedGraph};

use exec_common::istr;

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    )
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("catalog.test.one_of.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn run_write(
    graph: &SharedGraph,
    plan: &ExecutionPlan,
) -> Result<
    (
        BindingTable,
        Result<selene_graph::CommitOutcome, GraphError>,
    ),
    ExecutorError,
> {
    let snapshot = graph.read();
    let mut txn = graph.begin_write();
    let result = {
        let mut ctx = TxContext::write(
            snapshot,
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            &mut txn,
            graph.index_providers(),
        );
        execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
    };
    match result {
        Ok(table) => Ok((table, txn.commit())),
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

fn catalog_op(source: &str) -> PipelineOp {
    planned(source).pipeline.remove(0)
}

fn create_node_type_op(label: &str) -> PipelineOp {
    catalog_op(&format!("CREATE NODE TYPE :{label} ()"))
}

#[test]
fn create_edge_type_with_enumerated_from_resolves_to_oneof() {
    // Mnemosyne U12 reproducer: three single-label node types A, B, C; an edge
    // with `FROM :A, :B TO :C` must resolve to OneOf([idx_A, idx_B]) at source
    // and NodeType(idx_C) at target.
    let graph = empty_closed_graph(31100);
    let mut plan = planned("SHOW EDGE TYPES");
    plan.pipeline
        .insert(0, catalog_op("CREATE EDGE TYPE :E (FROM :A, :B TO :C)"));
    plan.pipeline.insert(0, create_node_type_op("C"));
    plan.pipeline.insert(0, create_node_type_op("B"));
    plan.pipeline.insert(0, create_node_type_op("A"));

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let graph_type = graph.graph_type().expect("closed graph type");
    let edge_type = &graph_type.edge_types[0];
    // A=0, B=1, C=2 (creation order). OneOf payload is sorted, so [0, 1].
    assert_eq!(
        edge_type.source_node_type,
        EdgeEndpointDef::OneOf(vec![0, 1])
    );
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(2));
}

#[test]
fn create_edge_type_with_eight_distinct_labels_resolves_to_oneof_spilled() {
    // Mnemosyne 8-label :WRITTEN_BY shape. Storage Vec<u32> heap-allocates
    // regardless; WAL SmallVec spills past inline cap 4. Verify the
    // gathered + sort+dedupe pipeline yields a stable, length-8 OneOf.
    let graph = empty_closed_graph(31101);
    let mut plan = planned("SHOW EDGE TYPES");
    plan.pipeline.insert(
        0,
        catalog_op(
            "CREATE EDGE TYPE :MENTIONS (FROM :Episode, :Fact, :Entity, :Skill, :BadPattern, :Note, :Session, :CoreBlock TO :Agent)",
        ),
    );
    for label in [
        "Agent",
        "CoreBlock",
        "Session",
        "Note",
        "BadPattern",
        "Skill",
        "Entity",
        "Fact",
        "Episode",
    ] {
        plan.pipeline.insert(0, create_node_type_op(label));
    }

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let graph_type = graph.graph_type().expect("closed graph type");
    let edge_type = &graph_type.edge_types[0];
    match &edge_type.source_node_type {
        EdgeEndpointDef::OneOf(indices) => {
            assert_eq!(indices.len(), 8, "all 8 distinct labels resolve");
            for window in indices.windows(2) {
                assert!(
                    window[0] < window[1],
                    "OneOf indices must be sorted: {:?}",
                    indices
                );
            }
        }
        other => panic!("expected OneOf source endpoint, got {other:?}"),
    }
    let agent_index = graph_type
        .node_type_index_for(istr("Agent"))
        .expect("Agent declared");
    assert_eq!(
        edge_type.target_node_type,
        EdgeEndpointDef::NodeType(agent_index)
    );
}

#[test]
fn create_edge_type_with_unknown_label_rejects() {
    // Row 4 of the cascade: any per-label miss emits GraphTypeViolation
    // mentioning the original label set.
    let graph = empty_closed_graph(31102);
    let mut plan = planned("SHOW EDGE TYPES");
    plan.pipeline.insert(
        0,
        catalog_op("CREATE EDGE TYPE :E (FROM :A, :Stranger TO :A)"),
    );
    plan.pipeline.insert(0, create_node_type_op("A"));

    let err = run_write(&graph, &plan).expect_err("unknown label rejected");
    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains(":Stranger") || message.contains("unknown node type label set")
    ));
}

#[test]
fn create_edge_type_with_single_label_does_not_produce_oneof() {
    // The cascade explicitly states single-label inputs never resolve to OneOf
    // — row 1 (exact match) wins. Guard against accidental degradation.
    let graph = empty_closed_graph(31103);
    let mut plan = planned("SHOW EDGE TYPES");
    plan.pipeline
        .insert(0, catalog_op("CREATE EDGE TYPE :E (FROM :A TO :A)"));
    plan.pipeline.insert(0, create_node_type_op("A"));

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.edge_types[0].source_node_type,
        EdgeEndpointDef::NodeType(0)
    );
}

#[test]
fn create_edge_type_with_repeated_label_collapses_to_node_type() {
    // Dedup: `FROM :A, :A TO :B` resolves to OneOf([0]) which the constructor
    // collapses to NodeType(0). Catches a regression where a careless cascade
    // skips the constructor and emits a malformed singleton OneOf.
    let graph = empty_closed_graph(31104);
    let mut plan = planned("SHOW EDGE TYPES");
    plan.pipeline
        .insert(0, catalog_op("CREATE EDGE TYPE :E (FROM :A, :A TO :B)"));
    plan.pipeline.insert(0, create_node_type_op("B"));
    plan.pipeline.insert(0, create_node_type_op("A"));

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.edge_types[0].source_node_type,
        EdgeEndpointDef::NodeType(0),
        "duplicated single label must collapse to NodeType via one_of constructor"
    );
}

#[test]
fn show_edge_types_round_trips_oneof_endpoint() {
    // F9 full round-trip: SHOW output PARSES and the re-executed DDL produces a
    // graph type bit-identical to the original. The renderer (commit 1) emits
    // OneOf as comma-joined node-type labels.
    let graph_a = empty_closed_graph(31200);
    let mut plan_a = planned("SHOW EDGE TYPES");
    plan_a
        .pipeline
        .insert(0, catalog_op("CREATE EDGE TYPE :E (FROM :A, :B TO :C)"));
    plan_a.pipeline.insert(0, create_node_type_op("C"));
    plan_a.pipeline.insert(0, create_node_type_op("B"));
    plan_a.pipeline.insert(0, create_node_type_op("A"));

    let (table_a, outcome_a) = run_write(&graph_a, &plan_a).expect("catalog A executes");
    outcome_a.expect("commit A succeeds");
    let Value::String(definition) = table_a.rows()[0].values()[1] else {
        panic!("definition is string");
    };
    let rendered = definition.as_str();
    assert_eq!(rendered, "CREATE EDGE TYPE :E (FROM :A,:B TO :C)");
    parse(rendered).expect("rendered OneOf DDL parses");

    let graph_b = empty_closed_graph(31201);
    let mut plan_b = planned("SHOW EDGE TYPES");
    plan_b.pipeline.insert(0, catalog_op(rendered));
    plan_b.pipeline.insert(0, create_node_type_op("C"));
    plan_b.pipeline.insert(0, create_node_type_op("B"));
    plan_b.pipeline.insert(0, create_node_type_op("A"));

    let (_table_b, outcome_b) = run_write(&graph_b, &plan_b).expect("catalog B executes");
    outcome_b.expect("commit B succeeds");

    let original_edge = &graph_a.graph_type().unwrap().edge_types[0];
    let replayed_edge = &graph_b.graph_type().unwrap().edge_types[0];
    assert_eq!(
        original_edge.source_node_type, replayed_edge.source_node_type,
        "source OneOf payload round-trips bit-identically"
    );
    assert_eq!(
        original_edge.target_node_type, replayed_edge.target_node_type,
        "target NodeType round-trips bit-identically"
    );
    assert_eq!(original_edge.label, replayed_edge.label);
}

#[test]
fn show_edge_types_round_trips_oneof_on_both_endpoints() {
    // Stronger round-trip: BOTH endpoints carry OneOf. Confirms the `(OneOf,
    // OneOf)` shape is renderable (per the commit-1 comment-update at
    // catalog/mod.rs render_edge_endpoint_clause) and survives re-parse.
    let graph_a = empty_closed_graph(31210);
    let mut plan_a = planned("SHOW EDGE TYPES");
    plan_a
        .pipeline
        .insert(0, catalog_op("CREATE EDGE TYPE :E (FROM :A, :B TO :C, :D)"));
    plan_a.pipeline.insert(0, create_node_type_op("D"));
    plan_a.pipeline.insert(0, create_node_type_op("C"));
    plan_a.pipeline.insert(0, create_node_type_op("B"));
    plan_a.pipeline.insert(0, create_node_type_op("A"));

    let (table_a, outcome_a) = run_write(&graph_a, &plan_a).expect("catalog A executes");
    outcome_a.expect("commit A succeeds");
    let Value::String(definition) = table_a.rows()[0].values()[1] else {
        panic!("definition is string");
    };
    let rendered = definition.as_str();
    assert_eq!(rendered, "CREATE EDGE TYPE :E (FROM :A,:B TO :C,:D)");
    parse(rendered).expect("dual-OneOf DDL parses");

    let graph_b = empty_closed_graph(31211);
    let mut plan_b = planned("SHOW EDGE TYPES");
    plan_b.pipeline.insert(0, catalog_op(rendered));
    plan_b.pipeline.insert(0, create_node_type_op("D"));
    plan_b.pipeline.insert(0, create_node_type_op("C"));
    plan_b.pipeline.insert(0, create_node_type_op("B"));
    plan_b.pipeline.insert(0, create_node_type_op("A"));
    let (_, outcome_b) = run_write(&graph_b, &plan_b).expect("catalog B executes");
    outcome_b.expect("commit B succeeds");

    let original_edge = &graph_a.graph_type().unwrap().edge_types[0];
    let replayed_edge = &graph_b.graph_type().unwrap().edge_types[0];
    assert_eq!(
        original_edge.source_node_type,
        replayed_edge.source_node_type
    );
    assert_eq!(
        original_edge.target_node_type,
        replayed_edge.target_node_type
    );
}
