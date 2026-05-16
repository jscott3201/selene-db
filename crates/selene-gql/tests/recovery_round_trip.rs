//! Runtime-produced WAL recovery round-trip tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{
    Change, GraphId, HlcTimestamp, LabelSet, NodeId, Origin, PropertyMap, Value, intern,
};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, TxContext, analyze, execute_pattern, execute_pipeline, parse, plan,
};
use selene_graph::{CommitOutcome, SharedGraph};
use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig, WalWriter};

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
        let input = if let Some(pattern) = &plan.pattern_plan {
            execute_pattern(pattern, &ctx)?
        } else {
            seed_table()
        };
        execute_pipeline(&plan.pipeline, input, &mut ctx)
    };
    match result {
        Ok(table) => {
            let outcome = txn.commit().expect("write commits");
            Ok((table, outcome))
        }
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-gql-recovery-{name}-{}-{nanos}",
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

fn node_ref(table: &BindingTable, column: &str) -> NodeId {
    let index = table
        .schema()
        .columns
        .iter()
        .position(|col| col.name.is_some_and(|name| name.as_str() == column))
        .expect("column exists");
    match table.rows()[0].get(index).expect("row value exists") {
        Value::NodeRef(id) => *id,
        other => panic!("expected node ref for {column}, got {other:?}"),
    }
}

#[test]
fn recover_from_wal_only_via_runtime_mutation_pipeline() {
    let dir = temp_dir("runtime-mutation");
    let graph_id = GraphId::new(9308);
    let graph = SharedGraph::new(graph_id);
    let plan =
        planned("INSERT (a:Person {name: 'alice'})-[:KNOWS]->(b:Person {name: 'bob'}) RETURN a, b");

    let (table, outcome) = run_write(&graph, &plan).expect("runtime write executes");
    let alice = node_ref(&table, "a");
    let bob = node_ref(&table, "b");
    append_wal(&dir, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let snapshot = recovered.read();
    let person = intern("Person").unwrap();
    let knows = intern("KNOWS").unwrap();
    let name = intern("name").unwrap();

    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeCreated { .. },
            Change::NodeCreated { .. },
            Change::EdgeCreated { .. }
        ]
    ));
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.edge_count(), 1);
    assert_eq!(snapshot.node_labels(alice), Some(&LabelSet::single(person)));
    assert_eq!(snapshot.node_labels(bob), Some(&LabelSet::single(person)));
    assert_eq!(
        snapshot
            .node_properties(alice)
            .and_then(|props| props.get(&name)),
        Some(&Value::String(intern("alice").unwrap()))
    );
    assert_eq!(
        snapshot
            .node_properties(bob)
            .and_then(|props| props.get(&name)),
        Some(&Value::String(intern("bob").unwrap()))
    );
    assert_eq!(
        snapshot.edge_label(selene_core::EdgeId::new(1)),
        Some(&knows)
    );
    assert_eq!(
        snapshot.edge_endpoints(selene_core::EdgeId::new(1)),
        Some((alice, bob))
    );
    assert_eq!(
        snapshot.edge_properties(selene_core::EdgeId::new(1)),
        Some(&PropertyMap::new())
    );
    let _ = fs::remove_dir_all(dir);
}
