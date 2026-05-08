//! Recovery orchestration.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use selene_core::{NodeId, Origin};

use crate::{
    DEFAULT_WAL_FILE_NAME, PersistError, PersistResult, ProviderRegistry, SectionEntry,
    SnapshotReader, WalReader, find_latest_snapshot,
};

/// Summary of a completed recovery pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryOutcome {
    /// Sequence of the snapshot whose bytes were applied, or `0` if none.
    pub applied_snapshot_seq: u64,
    /// Sequence of the last valid WAL entry read, or the snapshot sequence when
    /// no WAL file exists.
    pub last_wal_seq: u64,
    /// Provider tags that received at least one callback, sorted ascending.
    pub providers_invoked: Vec<[u8; 4]>,
    /// Number of WAL `Change` values delivered to provider fan-out.
    pub wal_changes_applied: u64,
    /// Number of replicated `Change` values skipped by `(source, sequence)`
    /// dedupe.
    pub replicated_changes_deduplicated: u64,
}

impl RecoveryOutcome {
    /// Outcome for a directory with no snapshot and no WAL.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            applied_snapshot_seq: 0,
            last_wal_seq: 0,
            providers_invoked: Vec::new(),
            wal_changes_applied: 0,
            replicated_changes_deduplicated: 0,
        }
    }
}

/// Run snapshot apply plus WAL replay against `dir`.
///
/// Snapshot sections are routed by provider tag to [`ProviderRegistry`]. WAL
/// entries after the applied snapshot sequence are decoded and fanned out to
/// every registered provider in deterministic provider-tag order.
///
/// # Errors
///
/// Returns persistence format errors, provider errors, or a
/// [`PersistError::WalSnapshotMismatch`] when the WAL does not extend the
/// applied snapshot epoch.
pub fn recover(dir: &Path, registry: &ProviderRegistry) -> PersistResult<RecoveryOutcome> {
    let mut outcome = RecoveryOutcome::empty();
    let mut providers_invoked = BTreeSet::new();

    outcome.applied_snapshot_seq = apply_snapshot(dir, registry, &mut providers_invoked)?;
    outcome.last_wal_seq = outcome.applied_snapshot_seq;
    replay_wal(dir, registry, &mut outcome, &mut providers_invoked)?;
    outcome.providers_invoked = providers_invoked.into_iter().collect();
    Ok(outcome)
}

fn apply_snapshot(
    dir: &Path,
    registry: &ProviderRegistry,
    providers_invoked: &mut BTreeSet<[u8; 4]>,
) -> PersistResult<u64> {
    let Some((snapshot_seq, path)) = find_latest_snapshot(dir)? else {
        return Ok(0);
    };
    let mut reader = SnapshotReader::open(&path)?;
    reader.verify_body_hash()?;
    let sections: Vec<SectionEntry> = reader.sections().to_vec();
    for (index, entry) in sections.iter().copied().enumerate() {
        let provider = registry
            .lookup(entry.provider)
            .ok_or(PersistError::UnknownProvider {
                provider: entry.provider,
                sub: entry.sub,
            })?;
        let bytes = reader.read_section_at(index)?;
        provider.read_section(entry.sub, &bytes).map_err(|source| {
            PersistError::ProviderFailed {
                provider: entry.provider,
                sub: Some(entry.sub),
                source,
            }
        })?;
        providers_invoked.insert(entry.provider);
    }
    Ok(snapshot_seq)
}

fn replay_wal(
    dir: &Path,
    registry: &ProviderRegistry,
    outcome: &mut RecoveryOutcome,
    providers_invoked: &mut BTreeSet<[u8; 4]>,
) -> PersistResult<()> {
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    if !wal_path.try_exists()? {
        return Ok(());
    }

    let reader = WalReader::open(&wal_path)?;
    if reader.snapshot_seq() != outcome.applied_snapshot_seq {
        return Err(PersistError::WalSnapshotMismatch {
            wal_snapshot_seq: reader.snapshot_seq(),
            snapshot_seq: outcome.applied_snapshot_seq,
        });
    }

    let applied_snapshot_seq = outcome.applied_snapshot_seq;
    let mut seen_source_seqs: HashMap<NodeId, HashSet<u64>> = HashMap::new();
    let stream = reader.iterate(move |header| header.sequence > applied_snapshot_seq)?;
    for view in stream {
        let view = match view {
            Ok(view) => view,
            Err(PersistError::TruncatedEntry { .. }) => break,
            Err(error) => return Err(error),
        };
        let entry = view.into_entry()?;
        outcome.last_wal_seq = entry.header.sequence;
        if skip_replicated_entry(
            entry.header.origin,
            entry.changes.len(),
            &mut seen_source_seqs,
            outcome,
        ) {
            continue;
        }
        for change in &entry.changes {
            for provider in registry.iter() {
                let tag = provider.provider_tag();
                provider
                    .on_change(change)
                    .map_err(|source| PersistError::ProviderFailed {
                        provider: tag,
                        sub: None,
                        source,
                    })?;
                providers_invoked.insert(tag);
            }
            outcome.wal_changes_applied = outcome.wal_changes_applied.saturating_add(1);
        }
    }
    Ok(())
}

fn skip_replicated_entry(
    origin: Origin,
    change_count: usize,
    seen_source_seqs: &mut HashMap<NodeId, HashSet<u64>>,
    outcome: &mut RecoveryOutcome,
) -> bool {
    let Origin::Replicated {
        source_node_id,
        source_seq,
    } = origin
    else {
        return false;
    };
    if seen_source_seqs
        .entry(source_node_id)
        .or_default()
        .insert(source_seq)
    {
        return false;
    }
    outcome.replicated_changes_deduplicated = outcome
        .replicated_changes_deduplicated
        .saturating_add(change_count as u64);
    true
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use proptest::prelude::*;
    use selene_core::{Change, HlcTimestamp, NodeId, Origin};

    use super::*;
    use crate::{
        RecoveryProvider, RecoveryResult, SectionCompression, SnapshotBuilder, SnapshotConfig,
        WalConfig, WalWriter,
    };

    #[derive(Clone, Debug, PartialEq)]
    enum Event {
        Section { sub: [u8; 4], bytes: Vec<u8> },
        Change(Box<Change>),
    }

    struct RecordingProvider {
        tag: [u8; 4],
        events: Mutex<Vec<Event>>,
        fail_section: bool,
        fail_change: bool,
    }

    impl RecordingProvider {
        fn new(tag: [u8; 4]) -> Arc<Self> {
            Self::with_failures(tag, false, false)
        }

        fn with_failures(tag: [u8; 4], fail_section: bool, fail_change: bool) -> Arc<Self> {
            Arc::new(Self {
                tag,
                events: Mutex::new(Vec::new()),
                fail_section,
                fail_change,
            })
        }

        fn events(&self) -> Vec<Event> {
            self.events.lock().unwrap().clone()
        }
    }

    impl RecoveryProvider for RecordingProvider {
        fn provider_tag(&self) -> [u8; 4] {
            self.tag
        }

        fn read_section(&self, sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()> {
            if self.fail_section {
                return Err(Box::new(TestProviderError("read_section failed")));
            }
            self.events.lock().unwrap().push(Event::Section {
                sub,
                bytes: bytes.to_vec(),
            });
            Ok(())
        }

        fn on_change(&self, change: &Change) -> RecoveryResult<()> {
            if self.fail_change {
                return Err(Box::new(TestProviderError("on_change failed")));
            }
            self.events
                .lock()
                .unwrap()
                .push(Event::Change(Box::new(change.clone())));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestProviderError(&'static str);

    impl fmt::Display for TestProviderError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl std::error::Error for TestProviderError {}

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "selene-recovery-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        dir
    }

    fn registry(providers: &[Arc<RecordingProvider>]) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for provider in providers {
            let provider: Arc<dyn RecoveryProvider> = provider.clone();
            registry.register(provider).unwrap();
        }
        registry
    }

    fn change(id: u64) -> Change {
        Change::NodeDeleted {
            id: NodeId::new(id),
        }
    }

    fn changes(ids: &[u64]) -> Vec<Change> {
        ids.iter().copied().map(change).collect()
    }

    fn write_snapshot(
        dir: &Path,
        sequence: u64,
        sections: &[([u8; 4], [u8; 4], Vec<u8>)],
    ) -> PathBuf {
        let mut builder = SnapshotBuilder::new(SnapshotConfig {
            dir: dir.to_path_buf(),
            sequence,
            compression: SectionCompression::None,
            fsync: true,
        });
        for (provider, sub, bytes) in sections {
            builder.add_section(*provider, *sub, bytes.clone()).unwrap();
        }
        builder.finalize().unwrap()
    }

    fn write_wal(dir: &Path, snapshot_seq: u64, entries: &[(Origin, Vec<Change>)]) -> PathBuf {
        let path = dir.join(DEFAULT_WAL_FILE_NAME);
        {
            let mut writer = WalWriter::open(
                &path,
                WalConfig {
                    snapshot_seq,
                    ..WalConfig::default()
                },
            )
            .unwrap();
            for (index, (origin, changes)) in entries.iter().enumerate() {
                writer
                    .append(
                        HlcTimestamp::new(index as u64 + 1, 0),
                        *origin,
                        None,
                        changes,
                    )
                    .unwrap();
            }
        }
        path
    }

    fn assert_provider_failed(
        error: PersistError,
        expected_provider: [u8; 4],
        expected_sub: Option<[u8; 4]>,
        expected_source: &str,
    ) {
        match error {
            PersistError::ProviderFailed {
                provider,
                sub,
                source,
            } => {
                assert_eq!(provider, expected_provider);
                assert_eq!(sub, expected_sub);
                assert_eq!(source.to_string(), expected_source);
            }
            other => panic!("expected provider failure, got {other:?}"),
        }
    }

    #[test]
    fn recover_empty_dir_returns_empty_outcome() {
        let dir = temp_dir("empty");
        let outcome = recover(&dir, &ProviderRegistry::new()).unwrap();
        assert_eq!(outcome, RecoveryOutcome::empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_snapshot_only_routes_sections() {
        let dir = temp_dir("snapshot-only");
        write_snapshot(
            &dir,
            12,
            &[
                (*b"CORE", *b"META", b"meta".to_vec()),
                (*b"VECT", *b"HNSW", b"hnsw".to_vec()),
            ],
        );
        let core = RecordingProvider::new(*b"CORE");
        let vector = RecordingProvider::new(*b"VECT");
        let outcome = recover(&dir, &registry(&[core.clone(), vector.clone()])).unwrap();
        assert_eq!(outcome.applied_snapshot_seq, 12);
        assert_eq!(outcome.last_wal_seq, 12);
        assert_eq!(outcome.providers_invoked, vec![*b"CORE", *b"VECT"]);
        assert_eq!(
            core.events(),
            vec![Event::Section {
                sub: *b"META",
                bytes: b"meta".to_vec()
            }]
        );
        assert_eq!(
            vector.events(),
            vec![Event::Section {
                sub: *b"HNSW",
                bytes: b"hnsw".to_vec()
            }]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_unknown_provider_is_rejected() {
        let dir = temp_dir("unknown");
        write_snapshot(&dir, 1, &[(*b"MISS", *b"BODY", b"body".to_vec())]);
        let core = RecordingProvider::new(*b"CORE");
        let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
        assert!(matches!(
            err,
            PersistError::UnknownProvider { provider, sub }
                if provider == *b"MISS" && sub == *b"BODY"
        ));
        assert!(core.events().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_wal_only_no_snapshot() {
        let dir = temp_dir("wal-only");
        write_wal(
            &dir,
            0,
            &[
                (Origin::Local, changes(&[1])),
                (Origin::Local, changes(&[2])),
                (Origin::Local, changes(&[3])),
            ],
        );
        let core = RecordingProvider::new(*b"CORE");
        let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
        assert_eq!(outcome.applied_snapshot_seq, 0);
        assert_eq!(outcome.last_wal_seq, 3);
        assert_eq!(outcome.wal_changes_applied, 3);
        assert_eq!(
            core.events(),
            changes(&[1, 2, 3])
                .into_iter()
                .map(|change| Event::Change(Box::new(change)))
                .collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_snapshot_and_wal_streams_only_post_snapshot_changes() {
        let dir = temp_dir("snapshot-wal");
        write_snapshot(&dir, 100, &[(*b"CORE", *b"META", b"meta".to_vec())]);
        write_wal(
            &dir,
            100,
            &[
                (Origin::Local, changes(&[101])),
                (Origin::Local, changes(&[102])),
                (Origin::Local, changes(&[103])),
            ],
        );
        let core = RecordingProvider::new(*b"CORE");
        let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
        assert_eq!(outcome.applied_snapshot_seq, 100);
        assert_eq!(outcome.last_wal_seq, 103);
        assert_eq!(outcome.wal_changes_applied, 3);
        assert_eq!(
            core.events(),
            vec![
                Event::Section {
                    sub: *b"META",
                    bytes: b"meta".to_vec()
                },
                Event::Change(Box::new(change(101))),
                Event::Change(Box::new(change(102))),
                Event::Change(Box::new(change(103))),
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_wal_snapshot_mismatch_is_rejected() {
        let dir = temp_dir("mismatch");
        write_snapshot(&dir, 50, &[]);
        write_wal(&dir, 100, &[]);
        let err = recover(&dir, &ProviderRegistry::new()).unwrap_err();
        assert!(matches!(
            err,
            PersistError::WalSnapshotMismatch {
                wal_snapshot_seq: 100,
                snapshot_seq: 50
            }
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_replicated_dedupe_skips_repeated_entries() {
        let dir = temp_dir("dedupe");
        let origin = Origin::Replicated {
            source_node_id: NodeId::new(9),
            source_seq: 77,
        };
        write_wal(
            &dir,
            0,
            &[(origin, changes(&[1, 2])), (origin, changes(&[3, 4]))],
        );
        let core = RecordingProvider::new(*b"CORE");
        let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
        assert_eq!(outcome.last_wal_seq, 2);
        assert_eq!(outcome.wal_changes_applied, 2);
        assert_eq!(outcome.replicated_changes_deduplicated, 2);
        assert_eq!(
            core.events(),
            vec![
                Event::Change(Box::new(change(1))),
                Event::Change(Box::new(change(2)))
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_provider_read_section_error_propagates() {
        let dir = temp_dir("section-fail");
        write_snapshot(&dir, 1, &[(*b"CORE", *b"META", b"meta".to_vec())]);
        write_wal(&dir, 1, &[(Origin::Local, changes(&[2]))]);
        let core = RecordingProvider::with_failures(*b"CORE", true, false);
        let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
        assert_provider_failed(err, *b"CORE", Some(*b"META"), "read_section failed");
        assert!(core.events().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_provider_on_change_error_propagates() {
        let dir = temp_dir("change-fail");
        write_wal(&dir, 0, &[(Origin::Local, changes(&[1]))]);
        let core = RecordingProvider::with_failures(*b"CORE", false, true);
        let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
        assert_provider_failed(err, *b"CORE", None, "on_change failed");
        assert!(core.events().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_truncated_wal_stops_at_last_valid_sequence() {
        let dir = temp_dir("truncated-wal");
        let path = write_wal(
            &dir,
            0,
            &[
                (Origin::Local, changes(&[1])),
                (Origin::Local, changes(&[2])),
                (Origin::Local, changes(&[3])),
            ],
        );
        let len = fs::metadata(&path).unwrap().len();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 1)
            .unwrap();
        let core = RecordingProvider::new(*b"CORE");
        let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
        assert_eq!(outcome.last_wal_seq, 2);
        assert_eq!(outcome.wal_changes_applied, 2);
        assert_eq!(
            core.events(),
            vec![
                Event::Change(Box::new(change(1))),
                Event::Change(Box::new(change(2)))
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recover_corrupt_snapshot_body_hash_is_hard_failure() {
        let dir = temp_dir("corrupt-snapshot");
        let path = write_snapshot(&dir, 1, &[(*b"CORE", *b"META", b"meta".to_vec())]);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 0x01;
        fs::write(&path, bytes).unwrap();
        let core = RecordingProvider::new(*b"CORE");
        let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
        assert!(matches!(err, PersistError::BodyHashMismatch { .. }));
        assert!(core.events().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    fn tag_strategy() -> impl Strategy<Value = [u8; 4]> {
        (65_u8..=90, 65_u8..=90, 65_u8..=90, 65_u8..=90).prop_map(|(a, b, c, d)| [a, b, c, d])
    }

    proptest! {
        #[test]
        fn recovery_replays_exact_change_sequence_to_every_provider(
            tags in proptest::collection::vec(tag_strategy(), 1..=32)
                .prop_filter("provider tags must be unique", |tags| {
                    let mut seen = std::collections::BTreeSet::new();
                    tags.iter().all(|tag| seen.insert(*tag))
                }),
            ids in proptest::collection::vec(1_u64..10_000, 1..=32),
        ) {
            let dir = temp_dir("prop-replay");
            let entries: Vec<_> = ids
                .iter()
                .copied()
                .map(|id| (Origin::Local, vec![change(id)]))
                .collect();
            write_wal(&dir, 0, &entries);
            let providers: Vec<_> = tags.iter().copied().map(RecordingProvider::new).collect();
            let outcome = recover(&dir, &registry(&providers)).unwrap();
            let expected_changes = changes(&ids);
            prop_assert_eq!(outcome.wal_changes_applied, expected_changes.len() as u64);
            for provider in providers {
                let observed: Vec<_> = provider
                    .events()
                    .into_iter()
                    .filter_map(|event| match event {
                        Event::Change(change) => Some(*change),
                        Event::Section { .. } => None,
                    })
                    .collect();
                prop_assert_eq!(observed.as_slice(), expected_changes.as_slice());
            }
            let _ = fs::remove_dir_all(dir);
        }
    }
}
