//! Recovery anchoring regressions for parent-directory aliases.

use std::io::Write;
use std::os::unix::fs::symlink;

use selene_persist::{
    AUDIT_KIND_RESERVED_0, AuditLog, AuditRecord, DEFAULT_AUDIT_FILE_NAME, DEFAULT_WAL_FILE_NAME,
    MANIFEST_FILE_NAME, WalReader,
};

use super::*;

#[test]
fn recovery_alias_retarget_cannot_redirect_the_reopened_writer() {
    let root = temp_dir("recovery-alias-retarget");
    let first_dir = root.join("first");
    let second_dir = root.join("second");
    let alias = root.join("live");
    fs::create_dir(&first_dir).unwrap();
    fs::create_dir(&second_dir).unwrap();
    symlink(&first_dir, &alias).unwrap();
    append_wal(&first_dir, 0, &[node_created(49)]);
    let first_audit = first_dir.join(DEFAULT_AUDIT_FILE_NAME);
    let second_audit = second_dir.join(DEFAULT_AUDIT_FILE_NAME);
    for path in [&first_audit, &second_audit] {
        let mut audit = AuditLog::open(path).unwrap();
        audit
            .append(&AuditRecord {
                recorded_at_unix_nanos: 1,
                kind: AUDIT_KIND_RESERVED_0,
                payload: vec![1, 2, 3],
            })
            .unwrap();
        drop(audit);
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&[0xAA, 0xBB, 0xCC])
            .unwrap();
    }
    let first_audit_torn_len = fs::metadata(&first_audit).unwrap().len();
    let second_audit_torn_len = fs::metadata(&second_audit).unwrap().len();

    let hook_alias = alias.clone();
    let hook_second = second_dir.clone();
    super::super::set_after_persist_recovery_hook(move || {
        fs::remove_file(&hook_alias).unwrap();
        symlink(&hook_second, &hook_alias).unwrap();
    });
    let recovered = SharedGraph::recover(&alias, GraphId::new(7)).unwrap();
    assert_eq!(
        alias.canonicalize().unwrap(),
        second_dir.canonicalize().unwrap()
    );
    assert_eq!(
        fs::metadata(&first_audit).unwrap().len(),
        first_audit_torn_len - 3
    );
    assert_eq!(
        fs::metadata(&second_audit).unwrap().len(),
        second_audit_torn_len
    );
    assert!(recovered.read().is_node_alive(NodeId::new(49)));
    let mut txn = recovered.begin_write();
    let created = txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    assert_eq!(created, NodeId::new(50));
    txn.commit().unwrap();
    drop(recovered);

    let sequences: Vec<_> = WalReader::open(&first_dir.join(DEFAULT_WAL_FILE_NAME))
        .unwrap()
        .iterate(|_| true)
        .unwrap()
        .map(|entry| entry.unwrap().header.sequence)
        .collect();
    assert_eq!(sequences, vec![1, 2]);
    assert!(!second_dir.join(DEFAULT_WAL_FILE_NAME).exists());
    assert!(!second_dir.join(MANIFEST_FILE_NAME).exists());

    fs::remove_dir_all(root).unwrap();
}
