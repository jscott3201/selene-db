//! End-to-end audit-log integration tests for BRIEF-Item-7 (separate audit.log,
//! audit Item 7 / Seam D, D24).
//!
//! These drive real commits through [`SharedGraph`] and assert that pack
//! lifecycle events are mirrored to a dedicated `audit.log` — durable across
//! restart and reattached on recovery — so pack history survives the WAL-archive
//! pruning (Item 5) that used to lose it (Seam D). The complementary proof that
//! `selene_persist::prune` never touches `audit.log` lives in
//! `selene-persist/src/retention/tests.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{GraphId, LabelSet, PackLifecycleEvent, PropertyMap, SchemaChange, intern};
use selene_graph::{GraphError, SharedGraph};
use selene_persist::{
    AUDIT_KIND_PACK_LIFECYCLE, AuditLog, DEFAULT_AUDIT_FILE_NAME, DEFAULT_WAL_FILE_NAME, WalConfig,
};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-audit-lifecycle-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).expect("temp dir created");
    dir
}

fn audit_path(dir: &Path) -> PathBuf {
    dir.join(DEFAULT_AUDIT_FILE_NAME)
}

/// A deterministic sample lifecycle event (the first `PackLifecycleEvent::ALL`
/// fixture).
fn sample_event() -> PackLifecycleEvent {
    PackLifecycleEvent::ALL[0]()
}

fn build_with_audit(dir: &Path, graph_id: GraphId) -> SharedGraph {
    SharedGraph::builder(graph_id)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .unwrap()
        .with_audit_log(audit_path(dir))
        .unwrap()
        .build()
        .unwrap()
}

fn commit_lifecycle(shared: &SharedGraph, graph_id: GraphId, event: PackLifecycleEvent) {
    let mut txn = shared.begin_write();
    txn.mutator()
        .schema_change(graph_id, SchemaChange::ProcedurePackLifecycle { event });
    txn.commit().unwrap();
}

#[test]
fn lifecycle_commit_mirrors_to_audit_log() {
    let dir = temp_dir("mirror");
    let graph_id = GraphId::new(200);
    let event = sample_event();
    let expected_payload = postcard::to_allocvec(&event).unwrap();

    {
        let shared = build_with_audit(&dir, graph_id);
        commit_lifecycle(&shared, graph_id, event);
    }

    let records = AuditLog::read_all(&audit_path(&dir)).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, AUDIT_KIND_PACK_LIFECYCLE);
    assert_eq!(records[0].payload, expected_payload);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn non_lifecycle_commit_writes_no_audit_record() {
    let dir = temp_dir("no-mirror");
    let graph_id = GraphId::new(201);
    {
        let shared = build_with_audit(&dir, graph_id);
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(intern("audit.node").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    // A plain node create is a graph change, not an engine event — no audit row.
    assert!(AuditLog::read_all(&audit_path(&dir)).unwrap().is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn audit_log_survives_restart_and_recovery_reattaches() {
    let dir = temp_dir("survive-recover");
    let graph_id = GraphId::new(202);

    // Live: commit one lifecycle event, then drop (releasing the WAL lock).
    {
        let shared = build_with_audit(&dir, graph_id);
        commit_lifecycle(&shared, graph_id, sample_event());
    }
    assert_eq!(AuditLog::read_all(&audit_path(&dir)).unwrap().len(), 1);

    // Recover: the historical event persists, and recovery reattaches the
    // audit log (the file is present) so post-recovery commits keep mirroring.
    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(
        AuditLog::read_all(&audit_path(&dir)).unwrap().len(),
        1,
        "recovery must not clobber or re-derive the audit log"
    );

    commit_lifecycle(&recovered, graph_id, sample_event());
    drop(recovered);
    assert_eq!(
        AuditLog::read_all(&audit_path(&dir)).unwrap().len(),
        2,
        "post-recovery lifecycle commit appends to the same audit log"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn audit_log_without_wal_is_rejected() {
    let dir = temp_dir("no-wal");
    let result = SharedGraph::builder(GraphId::new(203))
        .with_audit_log(audit_path(&dir))
        .unwrap()
        .build();
    assert!(
        matches!(result, Err(GraphError::Inconsistent { .. })),
        "audit without a WAL must be rejected"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn multiple_lifecycle_events_in_one_commit_all_mirror() {
    let dir = temp_dir("multi");
    let graph_id = GraphId::new(204);
    {
        let shared = build_with_audit(&dir, graph_id);
        let mut txn = shared.begin_write();
        // Two lifecycle events plus an unrelated node create in one transaction.
        txn.mutator().schema_change(
            graph_id,
            SchemaChange::ProcedurePackLifecycle {
                event: PackLifecycleEvent::ALL[0](),
            },
        );
        txn.mutator()
            .create_node(
                LabelSet::single(intern("audit.mixed").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        txn.mutator().schema_change(
            graph_id,
            SchemaChange::ProcedurePackLifecycle {
                event: PackLifecycleEvent::ALL[1](),
            },
        );
        txn.commit().unwrap();
    }
    // Exactly the two lifecycle events mirror; the node create does not.
    let records = AuditLog::read_all(&audit_path(&dir)).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.kind == AUDIT_KIND_PACK_LIFECYCLE));
    let _ = fs::remove_dir_all(dir);
}
