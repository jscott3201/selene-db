//! Composite-property index DDL and runtime coverage for BRIEF-140b.

mod exec_common;

use selene_core::{GraphId, IStr, LabelSet, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, JoinTree, NodeOrEdgeScan, ScanAccess, TxContext, analyze, execute_pattern,
    execute_pipeline, optimize, parse, plan,
};
use selene_graph::{CommitOutcome, GraphTypeDef, SharedGraph, TypedIndexKind};
use selene_testing::MockIndexCatalog;
use smallvec::smallvec;

use exec_common::{column_values, istr, props};

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn optimized(source: &str, catalog: &MockIndexCatalog) -> ExecutionPlan {
    let ctx = selene_gql::OptimizeContext::default().with_index_catalog(catalog);
    optimize(planned(source), &ctx)
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
            name: istr("catalog.composite.index.graph"),
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
) -> Result<(BindingTable, CommitOutcome), ExecutorError> {
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
        Ok(table) => Ok((table, txn.commit().expect("commit succeeds"))),
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

fn run_ddl(graph: &SharedGraph, source: &str) -> Result<CommitOutcome, ExecutorError> {
    run_write(graph, &planned(source)).map(|(_, outcome)| outcome)
}

fn graph_type_violation(graph: &SharedGraph, source: &str) -> String {
    let err = run_ddl(graph, source).expect_err("statement rejects");
    let ExecutorError::GraphTypeViolation { message, .. } = err else {
        panic!("expected GraphTypeViolation");
    };
    message
}

fn duplicate_name(graph: &SharedGraph, source: &str) -> IStr {
    let err = run_ddl(graph, source).expect_err("statement rejects");
    let ExecutorError::DuplicateObject { name, .. } = err else {
        panic!("expected DuplicateObject");
    };
    name
}

fn first_scan(tree: &JoinTree) -> Option<&NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. }
        | JoinTree::Repeat { child, .. }
        | JoinTree::PathSearch { child, .. } => first_scan(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan(left).or_else(|| first_scan(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

fn execute_read_plan(graph: &SharedGraph, plan: &ExecutionPlan) -> BindingTable {
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx).expect("pattern executes")
    } else {
        seed_table()
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx).expect("pipeline executes")
}

fn create_sensor_type(graph: &SharedGraph) {
    run_ddl(
        graph,
        "CREATE NODE TYPE :Sensor \
         (ts :: INT64, location :: STRING, value :: STRING, active :: BOOLEAN)",
    )
    .unwrap();
}

fn seeded_sensor_graph(id: u64, with_index: bool) -> SharedGraph {
    let graph = empty_closed_graph(id);
    create_sensor_type(&graph);
    let sensor = istr("Sensor");
    let ts = istr("ts");
    let location = istr("location");
    let value = istr("value");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(sensor.clone()),
                props([
                    (ts.clone(), Value::Int(1)),
                    (location.clone(), Value::String(istr("north"))),
                    (value.clone(), Value::String(istr("alpha"))),
                ]),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(sensor.clone()),
                props([
                    (ts.clone(), Value::Int(1)),
                    (location.clone(), Value::String(istr("south"))),
                    (value, Value::String(istr("beta"))),
                ]),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    if with_index {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_composite_property_index_named(
                sensor,
                smallvec![ts, location],
                smallvec![TypedIndexKind::I64, TypedIndexKind::String],
                Some(istr("sensor_ts_location_idx")),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    graph
}

fn column_strings(table: &BindingTable, name: &str) -> Vec<String> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::String(value) => value.as_str().to_owned(),
            other => panic!("expected string column {name}, got {other:?}"),
        })
        .collect()
}

#[test]
fn create_drop_and_same_property_set_idempotency_paths_work() {
    let graph = empty_closed_graph(14_101);
    create_sensor_type(&graph);

    run_ddl(&graph, "CREATE INDEX sensor_comp ON :Sensor(ts, location)").unwrap();
    let snapshot = graph.read();
    let rows = snapshot
        .iter_composite_property_index_entries()
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, istr("Sensor"));
    assert_eq!(rows[0].1.as_slice(), &[istr("ts"), istr("location")]);
    assert_eq!(
        rows[0].2.as_slice(),
        &[TypedIndexKind::I64, TypedIndexKind::String]
    );
    assert_eq!(rows[0].3, Some(istr("sensor_comp")));
    drop(snapshot);

    run_ddl(
        &graph,
        "CREATE INDEX IF NOT EXISTS other_name ON :Sensor(location, ts)",
    )
    .unwrap();
    assert_eq!(graph.read().composite_property_index_count(), 1);
    assert_eq!(
        duplicate_name(&graph, "CREATE INDEX other_name ON :Sensor(location, ts)").as_str(),
        "sensor_comp"
    );

    run_ddl(&graph, "DROP INDEX sensor_comp").unwrap();
    assert_eq!(graph.read().composite_property_index_count(), 0);
    run_ddl(&graph, "DROP INDEX IF EXISTS sensor_comp").unwrap();
    let missing = graph_type_violation(&graph, "DROP INDEX sensor_comp");
    assert!(missing.contains("index 'sensor_comp' does not exist"));
}

#[test]
fn same_property_set_duplicate_reports_existing_unnamed_declaration_order() {
    let graph = empty_closed_graph(14_107);
    create_sensor_type(&graph);
    let sensor = istr("Sensor");
    let ts = istr("ts");
    let location = istr("location");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_composite_property_index_named(
                sensor,
                smallvec![ts, location],
                smallvec![TypedIndexKind::I64, TypedIndexKind::String],
                None,
            )
            .unwrap();
        txn.commit().unwrap();
    }

    assert_eq!(
        duplicate_name(&graph, "CREATE INDEX requested ON :Sensor(location, ts)").as_str(),
        "idx:6:Sensor:c2:2:ts:8:location"
    );
}

#[test]
fn create_composite_rejects_duplicate_unsupported_and_edge_labels() {
    let graph = empty_closed_graph(14_102);
    create_sensor_type(&graph);
    run_ddl(&graph, "CREATE EDGE TYPE :Likes (score :: INT)").unwrap();

    let duplicate = graph_type_violation(&graph, "CREATE INDEX dup ON :Sensor(ts, ts)");
    assert!(duplicate.contains("contains duplicates"));
    assert!(duplicate.contains("ts"));

    let unsupported = graph_type_violation(&graph, "CREATE INDEX bad ON :Sensor(active, ts)");
    assert!(unsupported.contains("property kind Bool is not supported"));

    let edge = graph_type_violation(&graph, "CREATE INDEX edge_idx ON :Likes(score, score)");
    assert!(edge.contains("BRIEF-140c"));
}

#[test]
fn cross_map_name_collisions_and_ambiguous_drop_use_flat_namespace() {
    let graph = empty_closed_graph(14_103);
    create_sensor_type(&graph);
    run_ddl(&graph, "CREATE INDEX foo ON :Sensor(ts)").unwrap();
    assert_eq!(
        duplicate_name(&graph, "CREATE INDEX foo ON :Sensor(ts, location)").as_str(),
        "foo"
    );

    let graph = SharedGraph::new(GraphId::new(14_104));
    let sensor = istr("Sensor");
    let ts = istr("ts");
    let location = istr("location");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_property_index_named(
                sensor.clone(),
                ts.clone(),
                TypedIndexKind::I64,
                Some(istr("foo")),
            )
            .unwrap();
        mutator
            .create_composite_property_index_named(
                sensor,
                smallvec![ts, location],
                smallvec![TypedIndexKind::I64, TypedIndexKind::String],
                Some(istr("foo")),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let message = graph_type_violation(&graph, "DROP INDEX foo");
    assert!(message.contains("is ambiguous"));
    assert!(message.contains(":Sensor(ts)"));
    assert!(message.contains(":Sensor(ts, location)"));
    assert_eq!(graph.read().property_index_count(), 1);
    assert_eq!(graph.read().composite_property_index_count(), 1);
}

#[test]
fn optimized_composite_lookup_executes_against_live_storage() {
    let graph = seeded_sensor_graph(14_105, true);
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Sensor"),
        vec![
            (istr("ts"), selene_gql::IndexKind::Integer),
            (istr("location"), selene_gql::IndexKind::String),
        ],
    );
    let plan = optimized(
        "MATCH (s:Sensor) WHERE s.location = 'north' AND s.ts = 1 RETURN s.value AS value",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    let ScanAccess::CompositeLookup { properties, .. } = &scan.access else {
        panic!("expected composite lookup, got {:?}", scan.access);
    };
    let property_keys: Vec<IStr> = properties
        .iter()
        .map(|(property, _)| property.clone())
        .collect();
    assert_eq!(property_keys, vec![istr("ts"), istr("location")]);
    assert_eq!(
        graph
            .read()
            .composite_property_index_for(&istr("Sensor"), &property_keys)
            .unwrap()
            .cardinality(),
        2
    );

    let table = execute_read_plan(&graph, &plan);

    assert_eq!(column_strings(&table, "value"), ["alpha"]);
}

#[test]
fn optimized_composite_lookup_falls_back_when_storage_is_absent() {
    let graph = seeded_sensor_graph(14_106, false);
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Sensor"),
        vec![
            (istr("ts"), selene_gql::IndexKind::Integer),
            (istr("location"), selene_gql::IndexKind::String),
        ],
    );
    let plan = optimized(
        "MATCH (s:Sensor) WHERE s.location = 'north' AND s.ts = 1 RETURN s.value AS value",
        &catalog,
    );
    assert!(matches!(
        first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree)
            .unwrap()
            .access,
        ScanAccess::CompositeLookup { .. }
    ));

    let table = execute_read_plan(&graph, &plan);

    assert_eq!(column_strings(&table, "value"), ["alpha"]);
}
