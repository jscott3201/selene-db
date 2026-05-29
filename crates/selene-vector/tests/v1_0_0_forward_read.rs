//! Compatibility coverage for v1.0.0 vector snapshot/WAL bytes.
//!
//! BRIEF-Item-4a STEP 9 bumped the snapshot envelope minor version `0 -> 1`
//! (the `CORE/NODE` / `CORE/EDGE` format changed), so the two persisted lineages
//! diverge here:
//!   - the v1.0.0 **snapshot** (minor 0) is now cleanly REJECTED with
//!     `UnsupportedVersion` — the deliberate clean break (see
//!     `v1_0_0_snapshot_rejected_after_step9`);
//!   - the v1.0.0 **WAL** is unchanged by STEP 9 (the `Change` stream + WAL
//!     header are untouched), so it still forward-reads
//!     (`v1_0_0_wal_still_forward_reads`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, NodeId, Origin, intern};
use selene_graph::IndexProvider;
use selene_persist::{
    PersistError, SnapshotBuilder, SnapshotConfig, SnapshotReader, SyncPolicy, WalConfig,
    WalReader, WalWriter,
};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswIndexRegistry, IvfConfig, IvfIndexRegistry, PqParams,
    VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
};

const FIXTURE_DIR: &str = "crates/selene-testing/fixtures/v1_0_0_vector";
const SNAPSHOT_FILE: &str = "snapshot.1.snap";
const WAL_FILE: &str = "wal.log";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace parent")
        .parent()
        .expect("workspace root")
        .join(FIXTURE_DIR)
}

#[test]
fn v1_0_0_snapshot_rejected_after_step9() {
    // BRIEF-Item-4a STEP 9 clean break: the v1.0.0 snapshot carries envelope
    // minor version 0; opening it now fails the version gate with a clean
    // `UnsupportedVersion` (never a garbled mis-decode of the CORE sections).
    let snapshot_path = fixture_dir().join(SNAPSHOT_FILE);
    assert!(
        matches!(
            SnapshotReader::open(&snapshot_path),
            Err(PersistError::UnsupportedVersion { major: 1, minor: 0 })
        ),
        "pre-STEP-9 (minor 0) snapshot must be cleanly rejected"
    );
}

#[test]
fn v1_0_0_wal_still_forward_reads() {
    // The WAL lineage is untouched by STEP 9 (the `Change` stream + WAL header
    // are unchanged), so the v1.0.0 WAL still replays into the default vector
    // registries exactly as before.
    let wal_path = fixture_dir().join(WAL_FILE);

    let hnsw_from_wal = HnswIndexRegistry::new(hnsw_config()).expect("HNSW registry builds");
    let ivf_from_wal = IvfIndexRegistry::new(ivf_config()).expect("IVF registry builds");
    let wal = WalReader::open(&wal_path).expect("v1.0.0 WAL opens");
    for view in wal.iterate(|_| true).expect("WAL iterates") {
        let entry = view
            .expect("WAL entry reads")
            .into_entry()
            .expect("WAL entry CRC validates");
        for change in entry.changes {
            hnsw_from_wal
                .on_change(&change)
                .expect("legacy HNSW WAL routes to default");
            ivf_from_wal
                .on_change(&change)
                .expect("legacy IVF WAL routes to default");
        }
    }
    assert_eq!(
        hnsw_from_wal
            .get("default")
            .expect("default HNSW exists")
            .snapshot()
            .len(),
        1
    );
    assert_eq!(
        ivf_from_wal
            .get("default")
            .expect("default IVF exists")
            .snapshot()
            .len(),
        1
    );
}

#[test]
#[ignore = "fixture regeneration is manual; see _briefs/_audit/109-v1-0-0-fixture-recipe.md"]
fn regenerate_v1_0_0_vector_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace parent")
        .parent()
        .expect("workspace root");
    let dir = root.join(FIXTURE_DIR);
    fs::create_dir_all(&dir).expect("fixture dir exists");
    let _ = fs::remove_file(dir.join(SNAPSHOT_FILE));
    let _ = fs::remove_file(dir.join(format!("{SNAPSHOT_FILE}.tmp")));
    let _ = fs::remove_file(dir.join(WAL_FILE));
    write_snapshot_fixture(&dir);
    write_wal_fixture(&dir.join(WAL_FILE));
}

fn write_snapshot_fixture(dir: &Path) {
    let hnsw = selene_vector::HnswProvider::new(hnsw_config()).expect("HNSW provider builds");
    hnsw.on_change(&hnsw_change()).expect("HNSW change applies");
    let ivf = selene_vector::IvfProvider::new(ivf_config()).expect("IVF provider builds");
    ivf.on_change(&ivf_change()).expect("IVF change applies");

    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: PathBuf::from(dir),
        sequence: 1,
        compression: selene_persist::SectionCompression::None,
        fsync: false,
    });
    for sub_tag in [*b"GRPH", *b"VECS", *b"QUNT"] {
        builder
            .add_section(
                *b"VECT",
                sub_tag,
                hnsw.write_section(selene_graph::SubTag(sub_tag))
                    .expect("HNSW section writes"),
            )
            .expect("HNSW section adds");
    }
    for sub_tag in [*b"CQNT", *b"IPQB", *b"POST"] {
        builder
            .add_section(
                *b"IVFP",
                sub_tag,
                ivf.write_section(selene_graph::SubTag(sub_tag))
                    .expect("IVF section writes"),
            )
            .expect("IVF section adds");
    }
    builder.finalize().expect("snapshot fixture writes");
}

fn write_wal_fixture(path: &Path) {
    let mut writer = WalWriter::open(
        path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq: 0,
        },
    )
    .expect("WAL writer opens");
    writer
        .append(
            HlcTimestamp::zero(),
            Origin::Local,
            None,
            &[hnsw_change(), ivf_change()],
        )
        .expect("WAL append succeeds");
    writer.flush().expect("WAL flush succeeds");
}

fn hnsw_change() -> Change {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(1),
        vector: vec![1.0, 0.0, 0.0, 0.0],
        max_layer: 0,
    }
    .encode()
    .expect("VECU encodes");
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").expect("provider interns"),
        payload: Arc::from(payload.into_boxed_slice()),
    }
}

fn ivf_change() -> Change {
    let payload = VectorIvfUpsertV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(2),
        vector: vec![0.0, 1.0, 0.0, 0.0],
    }
    .encode()
    .expect("VIVF encodes");
    Change::IndexExtensionEvent {
        provider: intern("selene-vector-ivf").expect("provider interns"),
        payload: Arc::from(payload.into_boxed_slice()),
    }
}

fn hnsw_config() -> HnswConfig {
    HnswConfig::new(4).expect("HNSW config is valid")
}

fn ivf_config() -> IvfConfig {
    IvfConfig::with_params(
        4,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        256,
    )
    .expect("IVF config is valid")
}
