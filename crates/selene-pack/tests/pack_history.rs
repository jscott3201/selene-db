//! `selene.pack.history` integration tests.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jiff::{Timestamp, tz::TimeZone};
use selene_core::{
    Change, GraphId, HlcTimestamp, IStr, LabelSet, NodeId, Origin, PackLifecycleEvent, PropertyMap,
    SchemaChange, Value, intern,
};
use selene_gql::{
    AnalysisError, ExecutionPlan, ExecutorError, GqlType, ProcedureError, ProcedureMutability,
    ProcedureRegistry, ProcedureTier, Session, StatementOutput, analyze, execute_statement, parse,
    plan,
};
use selene_graph::SharedGraph;
use selene_pack::{PackHistorySource, ProcedurePackRegistry};
use selene_persist::{WalConfig, WalReader, WalWriter};

const GRAPH_ID: GraphId = GraphId::new(47);

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

struct FailingSource;

impl PackHistorySource for FailingSource {
    fn open_wal_reader(&self) -> Result<WalReader, ProcedureError> {
        Err(ProcedureError::Internal {
            detail: "history source unavailable".to_owned(),
        })
    }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn ts(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("test timestamp in range")
}

fn temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "selene-pack-history-{name}-{}-{nanos}.wal",
        std::process::id()
    ))
}

fn write_wal(entries: &[Vec<Change>]) -> PathBuf {
    let path = temp_path("wal");
    let mut writer = WalWriter::open(&path, WalConfig::default()).expect("test WAL opens");
    for (idx, changes) in entries.iter().enumerate() {
        writer
            .append(
                HlcTimestamp::new(idx as u64 + 1, 0),
                Origin::Local,
                None,
                changes,
            )
            .expect("test WAL append succeeds");
    }
    writer.flush().expect("test WAL flush succeeds");
    path
}

fn empty_wal() -> PathBuf {
    write_wal(&[])
}

fn lifecycle(event: PackLifecycleEvent) -> Change {
    Change::SchemaChanged {
        graph: GRAPH_ID,
        change: SchemaChange::ProcedurePackLifecycle { event },
    }
}

fn legacy(change: SchemaChange) -> Change {
    Change::SchemaChanged {
        graph: GRAPH_ID,
        change,
    }
}

fn node_created(id: u64) -> Change {
    Change::NodeCreated {
        id: NodeId::new(id),
        labels: LabelSet::single(istr("HistoryNode")),
        properties: PropertyMap::new(),
    }
}

fn staged(pack_name: &str, seconds: i64) -> PackLifecycleEvent {
    PackLifecycleEvent::Staged {
        pack_name: istr(pack_name),
        content_hash: [0_u8; 32],
        principal: istr("history.principal"),
        at: ts(seconds),
    }
}

fn activated(pack_name: &str, seconds: i64) -> PackLifecycleEvent {
    PackLifecycleEvent::Activated {
        pack_name: istr(pack_name),
        content_hash: [0_u8; 32],
        principal: istr("history.principal"),
        at: ts(seconds),
    }
}

fn deprecated(pack_name: &str, seconds: i64) -> PackLifecycleEvent {
    PackLifecycleEvent::Deprecated {
        pack_name: istr(pack_name),
        reason: istr("history.reason"),
        principal: istr("history.principal"),
        at: ts(seconds),
    }
}

fn disabled(pack_name: &str, seconds: i64) -> PackLifecycleEvent {
    PackLifecycleEvent::Disabled {
        pack_name: istr(pack_name),
        principal: istr("history.principal"),
        at: ts(seconds),
    }
}

fn validation_failed(pack_name: Option<&str>, error: &str, seconds: i64) -> PackLifecycleEvent {
    PackLifecycleEvent::ValidationFailed {
        pack_name: pack_name.map(istr),
        principal: istr("history.principal"),
        error: error.to_owned(),
        at: ts(seconds),
    }
}

fn registry_for_path(path: impl Into<PathBuf>) -> ProcedurePackRegistry {
    ProcedurePackRegistry::with_builtins_and_history(Arc::new(PathHistorySource::new(path)))
        .expect("platform built-ins register cleanly")
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

fn execute_history_rows(registry: &dyn ProcedureRegistry) -> Vec<Vec<Value>> {
    let graph = SharedGraph::new(GRAPH_ID);
    let plan = planned("CALL selene.pack.history() YIELD *", registry);
    let mut session = Session::new(&graph);
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

fn execute_history_err(registry: &dyn ProcedureRegistry) -> ExecutorError {
    let graph = SharedGraph::new(GRAPH_ID);
    let plan = planned("CALL selene.pack.history() YIELD *", registry);
    let mut session = Session::new(&graph);
    execute_statement(&plan, &mut session, registry).expect_err("history should fail")
}

fn kind(row: &[Value]) -> &'static str {
    let Value::String(value) = row[0] else {
        panic!("kind must be string");
    };
    value.as_str()
}

fn string_cell(row: &[Value], index: usize) -> Option<&'static str> {
    match row.get(index).expect("column index in range") {
        Value::String(value) => Some(value.as_str()),
        Value::Null => None,
        other => panic!("expected string or null, got {other:?}"),
    }
}

fn assert_internal_error(err: ExecutorError, label: &str) {
    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { detail },
            ..
        } if detail.contains(label)
    ));
}

fn placeholder_hash() -> String {
    format!("sha256:{}", "0".repeat(64))
}

#[test]
fn empty_wal_returns_zero_rows() {
    let path = empty_wal();
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert!(rows.is_empty());
    let _ = fs::remove_file(path);
}

#[test]
fn staged_event_returns_hash_row() {
    let path = write_wal(&[vec![lifecycle(staged("demo_pack", 1))]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert_eq!(rows.len(), 1);
    assert_eq!(kind(&rows[0]), "staged");
    assert_eq!(string_cell(&rows[0], 1), Some("demo_pack"));
    assert_eq!(string_cell(&rows[0], 2), Some(placeholder_hash().as_str()));
    let _ = fs::remove_file(path);
}

#[test]
fn full_lifecycle_returns_rows_in_wal_order() {
    let path = write_wal(&[
        vec![lifecycle(staged("demo_pack", 1))],
        vec![lifecycle(activated("demo_pack", 2))],
        vec![lifecycle(deprecated("demo_pack", 3))],
        vec![lifecycle(disabled("demo_pack", 4))],
    ]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    let kinds = rows.iter().map(|row| kind(row)).collect::<Vec<_>>();
    assert_eq!(kinds, ["staged", "activated", "deprecated", "disabled"]);
    let _ = fs::remove_file(path);
}

#[test]
fn validation_failed_without_pack_name_has_null_pack_name() {
    let path = write_wal(&[vec![lifecycle(validation_failed(None, "bad json", 1))]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert_eq!(rows.len(), 1);
    assert_eq!(kind(&rows[0]), "validation_failed");
    assert_eq!(string_cell(&rows[0], 1), None);
    assert_eq!(string_cell(&rows[0], 5), Some("bad json"));
    let _ = fs::remove_file(path);
}

#[test]
fn validation_failed_with_pack_name_populates_pack_name_and_error() {
    let path = write_wal(&[vec![lifecycle(validation_failed(
        Some("demo_pack"),
        "content hash mismatch",
        1,
    ))]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert_eq!(rows.len(), 1);
    assert_eq!(string_cell(&rows[0], 1), Some("demo_pack"));
    assert_eq!(string_cell(&rows[0], 5), Some("content hash mismatch"));
    let _ = fs::remove_file(path);
}

#[test]
fn mixed_wal_skips_non_lifecycle_and_legacy_variants() {
    let path = write_wal(&[vec![
        node_created(1),
        legacy(SchemaChange::ProcedurePackActivated {
            pack_name: istr("legacy_pack"),
            version: istr("1.0.0"),
        }),
        lifecycle(staged("modern_pack", 1)),
        legacy(SchemaChange::ProcedurePackDeprecated {
            pack_name: istr("legacy_pack"),
            version: istr("1.0.0"),
            reason: istr("legacy reason"),
        }),
        legacy(SchemaChange::ProcedurePackDisabled {
            pack_name: istr("legacy_pack"),
            version: istr("1.0.0"),
            reason: istr("legacy reason"),
        }),
    ]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert_eq!(rows.len(), 1);
    assert_eq!(kind(&rows[0]), "staged");
    assert_eq!(string_cell(&rows[0], 1), Some("modern_pack"));
    let _ = fs::remove_file(path);
}

#[test]
fn deprecated_row_carries_reason() {
    let path = write_wal(&[vec![lifecycle(deprecated("demo_pack", 1))]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert_eq!(kind(&rows[0]), "deprecated");
    assert_eq!(string_cell(&rows[0], 4), Some("history.reason"));
    let _ = fs::remove_file(path);
}

#[test]
fn disabled_row_has_null_reason() {
    let path = write_wal(&[vec![lifecycle(disabled("demo_pack", 1))]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    assert_eq!(kind(&rows[0]), "disabled");
    assert_eq!(string_cell(&rows[0], 4), None);
    let _ = fs::remove_file(path);
}

#[test]
fn source_error_surfaces_as_internal() {
    let registry = ProcedurePackRegistry::with_builtins_and_history(Arc::new(FailingSource))
        .expect("platform built-ins register cleanly");

    let err = execute_history_err(&registry);

    assert_internal_error(err, "history source unavailable");
}

#[test]
fn missing_wal_path_source_error_surfaces_as_internal() {
    let path = temp_path("missing");
    let registry = registry_for_path(&path);

    let err = execute_history_err(&registry);

    assert_internal_error(err, "pack history WAL open failed");
}

#[test]
fn with_builtins_only_does_not_register_pack_history() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let name = [istr("selene"), istr("pack"), istr("history")];

    assert!(registry.lookup(&name).is_none());
}

#[test]
fn with_builtins_and_history_registers_metadata() {
    let path = empty_wal();
    let registry = registry_for_path(&path);
    let name = [istr("selene"), istr("pack"), istr("history")];

    let metadata = registry.lookup(&name).expect("history is registered");

    assert_eq!(metadata.tier, ProcedureTier::Graph);
    assert_eq!(metadata.mutability, ProcedureMutability::Read);
    assert_eq!(metadata.signature.parameters.len(), 0);
    assert_eq!(metadata.output_schema.columns.len(), 7);
    assert_eq!(metadata.output_schema.columns[0].name.as_str(), "kind");
    assert_eq!(metadata.output_schema.columns[0].ty, GqlType::String);
    assert_eq!(metadata.output_schema.columns[6].name.as_str(), "at");
    assert_eq!(metadata.output_schema.columns[6].ty, GqlType::ZonedDateTime);
    let _ = fs::remove_file(path);
}

#[test]
fn multi_entry_wal_preserves_sequence_order() {
    let path = write_wal(&[
        vec![lifecycle(staged("first_pack", 1))],
        vec![lifecycle(staged("second_pack", 2))],
        vec![lifecycle(staged("third_pack", 3))],
    ]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    let names = rows
        .iter()
        .map(|row| string_cell(row, 1).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["first_pack", "second_pack", "third_pack"]);
    let _ = fs::remove_file(path);
}

#[test]
fn at_column_is_utc_zoned_datetime() {
    let timestamp = ts(1_700_000_000);
    let path = write_wal(&[vec![lifecycle(PackLifecycleEvent::Staged {
        pack_name: istr("demo_pack"),
        content_hash: [0_u8; 32],
        principal: istr("history.principal"),
        at: timestamp,
    })]]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    let Value::ZonedDateTime(zoned) = &rows[0][6] else {
        panic!("at column must be zoned datetime");
    };
    assert_eq!(zoned.time_zone(), &TimeZone::UTC);
    assert_eq!(zoned.timestamp(), timestamp);
    let _ = fs::remove_file(path);
}

#[test]
fn output_column_count_and_names_match_schema() {
    let path = empty_wal();
    let registry = registry_for_path(&path);
    let metadata = registry
        .lookup(&[istr("selene"), istr("pack"), istr("history")])
        .unwrap();

    let names = metadata
        .output_schema
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "kind",
            "pack_name",
            "content_hash",
            "principal",
            "reason",
            "error",
            "at"
        ]
    );
    let _ = fs::remove_file(path);
}

#[test]
fn wal_iterate_failure_surfaces_as_internal() {
    let path = write_wal(&[vec![lifecycle(staged("demo_pack", 1))]]);
    let len = fs::metadata(&path).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(len - 1)
        .unwrap();
    let registry = registry_for_path(&path);

    let err = execute_history_err(&registry);

    assert_internal_error(err, "pack history WAL iterate failed");
    let _ = fs::remove_file(path);
}

#[test]
fn wal_entry_decode_failure_surfaces_as_internal() {
    let path = write_wal(&[vec![lifecycle(staged("demo_pack", 1))]]);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte[0] ^ 0xff]).unwrap();
    file.flush().unwrap();
    let registry = registry_for_path(&path);

    let err = execute_history_err(&registry);

    assert_internal_error(err, "pack history WAL entry decode failed");
    let _ = fs::remove_file(path);
}

#[test]
fn gql_call_unknown_pack_history_surfaces_analysis_error() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let statement = parse("CALL selene.pack.history() YIELD *").unwrap();

    let err = analyze(statement, &registry, None).expect_err("history is not registered");

    assert!(matches!(err, AnalysisError::UnknownProcedure { .. }));
}

#[test]
fn null_columns_match_event_kind_table() {
    let path = write_wal(&[
        vec![lifecycle(validation_failed(None, "bad", 1))],
        vec![lifecycle(staged("demo_pack", 2))],
        vec![lifecycle(activated("demo_pack", 3))],
        vec![lifecycle(deprecated("demo_pack", 4))],
        vec![lifecycle(disabled("demo_pack", 5))],
    ]);
    let registry = registry_for_path(&path);

    let rows = execute_history_rows(&registry);

    let patterns = rows
        .iter()
        .map(|row| {
            (
                kind(row),
                matches!(row[1], Value::Null),
                matches!(row[2], Value::Null),
                matches!(row[4], Value::Null),
                matches!(row[5], Value::Null),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        patterns,
        [
            ("validation_failed", true, true, true, false),
            ("staged", false, false, true, true),
            ("activated", false, false, true, true),
            ("deprecated", false, true, false, true),
            ("disabled", false, true, true, true),
        ]
    );
    let _ = fs::remove_file(path);
}
