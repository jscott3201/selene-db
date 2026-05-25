//! Named index DDL executor coverage for BRIEF-140a.

mod exec_common;

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use selene_core::{
    Change, GraphId, HlcTimestamp, IStr, Origin, Value,
    feature_register::{FeatureId, SUPPORTED_FEATURES},
};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, Session, StatementOutput, TxContext, analyze, execute_pipeline, feature_walk,
    parse, plan,
};
use selene_graph::{CommitOutcome, GraphTypeDef, SharedGraph, TypedIndexKind};
use selene_pack::ProcedurePackRegistry;
use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig, WalWriter};

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
        .bound_to(empty_graph_type())
        .unwrap()
        .build()
        .unwrap()
}

fn empty_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("catalog.index.graph"),
        node_types: Vec::new(),
        edge_types: Vec::new(),
    }
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

fn show_indexes(graph: &SharedGraph) -> BindingTable {
    run_write(graph, &planned("SHOW INDEXES"))
        .expect("SHOW INDEXES executes")
        .0
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

fn index_entry(
    graph: &SharedGraph,
    label: &str,
    property: &str,
) -> Option<(TypedIndexKind, Option<IStr>)> {
    graph
        .read()
        .iter_property_index_entries()
        .find(|(entry_label, entry_property, _, _)| {
            entry_label.as_str() == label && entry_property.as_str() == property
        })
        .map(|(_, _, kind, name)| (kind, name))
}

fn column_strings(table: &BindingTable, column: &str) -> Vec<String> {
    let index = table
        .column_index(istr(column))
        .unwrap_or_else(|| panic!("missing column {column}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            Some(Value::ExternalString(value)) => value.as_ref().to_owned(),
            other => panic!("expected string column {column}, got {other:?}"),
        })
        .collect()
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-gql-index-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn append_wal(dir: &Path, changes: &[Change]) {
    let mut writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq: 0,
        },
    )
    .unwrap();
    writer
        .append(HlcTimestamp::zero(), Origin::Local, None, changes)
        .unwrap();
    writer.flush().unwrap();
}

#[test]
fn create_show_drop_and_idempotency_paths_work() {
    let graph = empty_closed_graph(14_001);
    run_ddl(
        &graph,
        "CREATE NODE TYPE :Sensor (ts :: LOCAL DATETIME, value :: STRING)",
    )
    .unwrap();

    run_ddl(&graph, "CREATE INDEX sensor_ts_idx ON :Sensor(ts)").unwrap();
    assert_eq!(
        index_entry(&graph, "Sensor", "ts"),
        Some((TypedIndexKind::LocalDateTime, Some(istr("sensor_ts_idx"))))
    );
    run_ddl(
        &graph,
        "CREATE INDEX IF NOT EXISTS sensor_ts_idx ON :Sensor(ts)",
    )
    .unwrap();

    assert_eq!(
        duplicate_name(&graph, "CREATE INDEX sensor_ts_idx ON :Sensor(ts)").as_str(),
        "sensor_ts_idx"
    );
    assert_eq!(
        column_strings(&show_indexes(&graph), "name"),
        ["sensor_ts_idx"]
    );

    run_ddl(&graph, "DROP INDEX sensor_ts_idx").unwrap();
    assert_eq!(graph.read().property_index_count(), 0);
    run_ddl(&graph, "DROP INDEX IF EXISTS sensor_ts_idx").unwrap();
    let missing = graph_type_violation(&graph, "DROP INDEX sensor_ts_idx");
    assert!(missing.contains("index 'sensor_ts_idx' does not exist"));
}

#[test]
fn create_index_rejects_deferred_or_invalid_shapes_with_precise_errors() {
    let graph = empty_closed_graph(14_002);
    run_ddl(
        &graph,
        "CREATE NODE TYPE :Sensor (ts :: LOCAL DATETIME, value :: STRING, \
         tags :: LIST<STRING>, zdt :: ZONED DATETIME, active :: BOOLEAN)",
    )
    .unwrap();
    run_ddl(&graph, "CREATE EDGE TYPE :Likes (score :: INT)").unwrap();

    let composite = graph_type_violation(&graph, "CREATE INDEX composite ON :Sensor(ts, value)");
    assert!(composite.contains("BRIEF-140b"));
    let edge = graph_type_violation(&graph, "CREATE INDEX edge_idx ON :Likes(score)");
    assert!(edge.contains("BRIEF-140c"));
    let missing_label = graph_type_violation(&graph, "CREATE INDEX missing_idx ON :Missing(ts)");
    assert!(missing_label.contains("node type ':Missing' is not declared"));
    assert!(!missing_label.contains("BRIEF-140c"));
    let missing_property =
        graph_type_violation(&graph, "CREATE INDEX missing_prop ON :Sensor(nope)");
    assert!(missing_property.contains("property 'nope' is not declared on type ':Sensor'"));

    for (source, expected) in [
        (
            "CREATE INDEX list_idx ON :Sensor(tags)",
            "property kind List is not supported",
        ),
        (
            "CREATE INDEX zdt_idx ON :Sensor(zdt)",
            "property kind ZonedDateTime is not supported",
        ),
        (
            "CREATE INDEX bool_idx ON :Sensor(active)",
            "property kind Bool is not supported",
        ),
    ] {
        let message = graph_type_violation(&graph, source);
        assert!(message.contains(expected), "{source}: {message}");
    }
}

#[test]
fn create_index_infers_all_supported_storage_kinds() {
    let graph = empty_closed_graph(14_003);
    run_ddl(
        &graph,
        "CREATE NODE TYPE :T \
         (i :: INT64, f :: FLOAT64, s :: STRING, d :: DATE, ldt :: LOCAL DATETIME, u :: UUID)",
    )
    .unwrap();

    for (name, property, kind) in [
        ("t_i", "i", TypedIndexKind::I64),
        ("t_f", "f", TypedIndexKind::F64),
        ("t_s", "s", TypedIndexKind::String),
        ("t_d", "d", TypedIndexKind::Date),
        ("t_ldt", "ldt", TypedIndexKind::LocalDateTime),
        ("t_u", "u", TypedIndexKind::Uuid),
    ] {
        run_ddl(&graph, &format!("CREATE INDEX {name} ON :T({property})")).unwrap();
        assert_eq!(
            index_entry(&graph, "T", property),
            Some((kind, Some(istr(name))))
        );
    }
}

#[test]
fn named_index_survives_wal_recovery() {
    let dir = temp_dir("wal");
    let graph_id = GraphId::new(14_004);
    let base = empty_graph_type();
    let graph = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let mut changes = Vec::new();
    changes.extend(
        run_ddl(&graph, "CREATE NODE TYPE :Sensor (ts :: LOCAL DATETIME)")
            .unwrap()
            .changes,
    );
    changes.extend(
        run_ddl(&graph, "CREATE INDEX sensor_ts_idx ON :Sensor(ts)")
            .unwrap()
            .changes,
    );
    append_wal(&dir, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    assert_eq!(
        index_entry(&recovered, "Sensor", "ts"),
        Some((TypedIndexKind::LocalDateTime, Some(istr("sensor_ts_idx"))))
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn flagger_records_im_index_ddl_and_feature_is_supported() {
    assert!(SUPPORTED_FEATURES.contains(&FeatureId::IM_INDEX_DDL));

    for source in [
        "CREATE INDEX sensor_ts_idx ON :Sensor(ts)",
        "DROP INDEX sensor_ts_idx",
    ] {
        let statement = parse(source).expect("statement parses");
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(observed.contains(&FeatureId::IM_INDEX_DDL), "{observed:?}");
    }
}

#[test]
fn procedure_indexes_and_ddl_indexes_share_catalog_names() {
    let graph = empty_closed_graph(14_005);
    let registry = ProcedurePackRegistry::with_builtins().expect("built-ins register");
    run_ddl(
        &graph,
        "CREATE NODE TYPE :Sensor (ts :: LOCAL DATETIME, value :: STRING)",
    )
    .unwrap();
    run_ddl(&graph, "CREATE INDEX ddl_value_idx ON :Sensor(value)").unwrap();

    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CALL selene.create_index('Sensor', 'ts', 'local_datetime')",
            &registry,
        )
        .expect("procedure index creates");
    let table = rows(
        session
            .execute_source("SHOW INDEXES", &registry)
            .expect("SHOW INDEXES executes"),
    );
    let names = column_strings(&table, "name");
    assert!(names.contains(&"ddl_value_idx".to_owned()));
    assert!(names.contains(&"idx:6:Sensor:2:ts".to_owned()));

    let collision = duplicate_name(&graph, "CREATE INDEX `idx:6:Sensor:2:ts` ON :Sensor(value)");
    assert_eq!(collision.as_str(), "idx:6:Sensor:2:ts");
    run_ddl(&graph, "DROP INDEX `idx:6:Sensor:2:ts`").unwrap();
    assert!(index_entry(&graph, "Sensor", "ts").is_none());
}

#[test]
fn duplicate_names_and_same_pair_conflicts_follow_brief_matrix() {
    let graph = empty_closed_graph(14_006);
    run_ddl(
        &graph,
        "CREATE NODE TYPE :Sensor (ts :: LOCAL DATETIME, value :: STRING)",
    )
    .unwrap();
    run_ddl(&graph, "CREATE INDEX foo ON :Sensor(ts)").unwrap();

    assert_eq!(
        duplicate_name(&graph, "CREATE INDEX foo ON :Sensor(value)").as_str(),
        "foo"
    );
    assert_eq!(column_strings(&show_indexes(&graph), "name"), ["foo"]);
    run_ddl(&graph, "CREATE INDEX IF NOT EXISTS new_name ON :Sensor(ts)").unwrap();
    assert_eq!(
        index_entry(&graph, "Sensor", "ts"),
        Some((TypedIndexKind::LocalDateTime, Some(istr("foo"))))
    );
    assert_eq!(
        duplicate_name(&graph, "CREATE INDEX new_name ON :Sensor(ts)").as_str(),
        "foo"
    );
}

#[test]
fn drop_index_refuses_ambiguous_rendered_name_matches() {
    let graph = SharedGraph::new(GraphId::new(14_007));
    let sensor = istr("Sensor");
    let meter = istr("Meter");
    let ts = istr("ts");
    let collision = istr("colliding_name");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_property_index_named(sensor, ts, TypedIndexKind::I64, Some(collision))
            .unwrap();
        mutator
            .create_property_index_named(meter, ts, TypedIndexKind::I64, Some(collision))
            .unwrap();
        txn.commit().unwrap();
    }

    let message = graph_type_violation(&graph, "DROP INDEX colliding_name");
    assert!(message.contains("is ambiguous"));
    assert!(message.contains(":Sensor(ts)"));
    assert!(message.contains(":Meter(ts)"));
    assert_eq!(graph.read().property_index_count(), 2);
}
