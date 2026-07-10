use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use selene_core::GraphId;
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig, WalWriter};

use super::*;

#[test]
fn bare_relative_wal_uses_current_directory_as_checkpoint_target() {
    assert_eq!(
        checkpoint_dir(Path::new(DEFAULT_WAL_FILE_NAME)),
        Path::new(".")
    );
}

#[test]
fn generation_aware_core_snapshot_rejects_a_different_generation() {
    let mut graph = SeleneGraph::new(GraphId::new(91_001));
    graph.meta.generation = 7;
    let provider = CoreProvider::new_for_live(Arc::new(ArcSwap::from_pointee(graph)));

    IndexProvider::write_section_at_generation(provider.as_ref(), SubTag(CORE_META_SUB), 7)
        .expect("matching published generation encodes");
    assert!(matches!(
        IndexProvider::write_section_at_generation(
            provider.as_ref(),
            SubTag(CORE_META_SUB),
            6,
        ),
        Err(ProviderError::Inconsistent { reason })
            if reason.contains("generation 7") && reason.contains("generation 6")
    ));
}

#[test]
fn checkpoint_target_requires_live_nonzero_default_wal() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(
        91_002,
    ))));
    let no_wal = CoreProvider::new_for_live(Arc::clone(&snapshot));
    assert!(matches!(
        no_wal.checkpoint_target(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("opened with a WAL")
    ));

    let zero_path = temp_wal_path("checkpoint-zero");
    let zero_writer = WalWriter::open(&zero_path, WalConfig::default()).unwrap();
    let zero = CoreProvider::new_for_live_with_wal(
        Arc::clone(&snapshot),
        Some(DurableState::new(zero_writer)),
    );
    assert!(matches!(
        zero.checkpoint_target(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("nonzero durable WAL")
    ));

    let custom_path = zero_path.parent().unwrap().join("custom-checkpoint.wal");
    let custom_writer = WalWriter::open(
        &custom_path,
        WalConfig {
            snapshot_seq: 3,
            ..WalConfig::default()
        },
    )
    .unwrap();
    let custom = CoreProvider::new_for_live_with_wal(
        Arc::clone(&snapshot),
        Some(DurableState::new(custom_writer)),
    );
    assert!(matches!(
        custom.checkpoint_target(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains(DEFAULT_WAL_FILE_NAME) && reason.contains("custom-checkpoint.wal")
    ));

    let ready_path = temp_wal_path("checkpoint-ready");
    let ready_writer = WalWriter::open(
        &ready_path,
        WalConfig {
            snapshot_seq: 9,
            ..WalConfig::default()
        },
    )
    .unwrap();
    let ready =
        CoreProvider::new_for_live_with_wal(snapshot, Some(DurableState::new(ready_writer)));
    let target = ready.checkpoint_target().expect("default WAL target");
    assert_eq!(target.sequence, 9);
    assert_eq!(
        target.dir,
        ready_path.parent().unwrap().canonicalize().unwrap()
    );
}
