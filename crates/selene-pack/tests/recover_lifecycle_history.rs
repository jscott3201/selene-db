//! Procedure-pack lifecycle WAL recovery and history tests.

mod common;

use common::manifest_fixture::build_manifest_json_with_canonical_hash;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use jiff::Timestamp;
use selene_core::{Change, GraphId, HlcTimestamp, IStr, Origin, intern};
use selene_gql::{
    ExecutionPlan, ProcedureError, ProcedureRegistry, Session, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_pack::{
    ActivationRegistry, GraphCommitSink, PackHistorySource, Principal, ProcedurePackRegistry,
    Uploaded, Value,
};
use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig, WalReader, WalWriter};

struct WalAppendingProvider {
    writer: Mutex<WalWriter>,
}

impl WalAppendingProvider {
    fn new(path: &Path) -> Self {
        let writer = WalWriter::open(
            path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(1),
                snapshot_seq: 0,
            },
        )
        .expect("test WAL opens");
        Self {
            writer: Mutex::new(writer),
        }
    }

    fn flush(&self) {
        self.writer.lock().expect("wal lock").flush().unwrap();
    }
}

impl IndexProvider for WalAppendingProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"WALP")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.writer
            .lock()
            .expect("wal lock")
            .append(
                HlcTimestamp::zero(),
                Origin::Local,
                None,
                std::slice::from_ref(change),
            )
            .map(|_| ())
            .map_err(|error| ProviderError::Inconsistent {
                reason: error.to_string(),
            })
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[derive(Clone)]
struct PathHistorySource {
    path: Arc<PathBuf>,
}

impl PathHistorySource {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::new(path.into()),
        }
    }
}

impl PackHistorySource for PathHistorySource {
    fn open_wal_reader(&self) -> Result<WalReader, ProcedureError> {
        WalReader::open(&self.path).map_err(|source| ProcedureError::Internal {
            detail: format!("pack history WAL open failed: {source}"),
        })
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-pack-recover-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn ts(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("test timestamp in range")
}

fn principal() -> Principal {
    Principal::new(istr("recover.history.principal"))
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

fn execute_history_rows(graph: &SharedGraph, registry: &dyn ProcedureRegistry) -> Vec<Vec<Value>> {
    let plan = planned("CALL selene.pack.history() YIELD *", registry);
    let mut session = Session::new(graph);
    let output = execute_statement(&plan, &mut session, registry).expect("history executes");
    let StatementOutput::Rows(table) = output else {
        panic!("expected history rows");
    };
    table
        .rows()
        .iter()
        .map(|row| row.values().to_vec())
        .collect()
}

fn string_cell(row: &[Value], index: usize) -> Option<&str> {
    match row.get(index).expect("column index in range") {
        Value::String(value) => Some(value.as_str()),
        Value::ExternalString(value) => Some(value.as_ref()),
        Value::Null => None,
        other => panic!("expected string or null, got {other:?}"),
    }
}

#[test]
fn selene_pack_history_round_trips_through_recover() {
    let dir = temp_dir("history");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let graph_id = GraphId::new(9341);
    let wal = Arc::new(WalAppendingProvider::new(&wal_path));
    let graph = Arc::new(
        SharedGraph::builder(graph_id)
            .with_provider(wal.clone())
            .build()
            .unwrap(),
    );
    let sink = GraphCommitSink::new(Arc::clone(&graph), graph_id);
    let mut activation_registry = ActivationRegistry::new();

    let _active = Uploaded::new(
        build_manifest_json_with_canonical_hash("recover_pack"),
        principal(),
        ts(1),
    )
    .validate(&sink, ts(2))
    .expect("manifest validates")
    .stage(&sink, ts(3))
    .expect("pack stages")
    .activate(&sink, &mut activation_registry, ts(4))
    .expect("pack activates");
    wal.flush();

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let registry = ProcedurePackRegistry::with_builtins_and_history(Arc::new(
        PathHistorySource::new(wal_path),
    ))
    .unwrap();
    let rows = execute_history_rows(&recovered, &registry);

    assert_eq!(rows.len(), 2);
    assert_eq!(string_cell(&rows[0], 0), Some("staged"));
    assert_eq!(string_cell(&rows[0], 1), Some("recover_pack"));
    assert_eq!(string_cell(&rows[1], 0), Some("activated"));
    assert_eq!(string_cell(&rows[1], 1), Some("recover_pack"));
    assert_eq!(string_cell(&rows[1], 3), Some("recover.history.principal"));
    assert_eq!(recovered.read().node_count(), 0);
    assert_eq!(recovered.read().edge_count(), 0);
    let _ = fs::remove_dir_all(dir);
}
