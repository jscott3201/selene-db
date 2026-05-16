//! Forward-read coverage for v1.0.0 vector snapshot/WAL bytes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, NodeId, Origin, intern};
use selene_graph::IndexProvider;
use selene_persist::{
    SnapshotBuilder, SnapshotConfig, SnapshotReader, SyncPolicy, WalConfig, WalReader, WalWriter,
};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswIndexRegistry, IvfConfig, IvfIndexRegistry, PqParams,
    VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
};

const FIXTURE_DIR: &str = "crates/selene-testing/fixtures/v1_0_0_vector";
const SNAPSHOT_FILE: &str = "snapshot.1.snap";
const WAL_FILE: &str = "wal.log";

#[test]
fn v1_0_0_snapshot_and_wal_forward_read_as_default() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace parent")
        .parent()
        .expect("workspace root")
        .join(FIXTURE_DIR);
    let snapshot_path = fixture_dir.join(SNAPSHOT_FILE);
    let wal_path = fixture_dir.join(WAL_FILE);

    let hnsw_from_snapshot = HnswIndexRegistry::new(hnsw_config()).expect("HNSW registry builds");
    let ivf_from_snapshot = IvfIndexRegistry::new(ivf_config()).expect("IVF registry builds");
    let mut reader = SnapshotReader::open(&snapshot_path).expect("v1.0.0 snapshot opens");
    for sub_tag in [*b"GRPH", *b"VECS", *b"QUNT"] {
        let bytes = reader
            .read_section(*b"VECT", sub_tag)
            .expect("legacy HNSW section reads");
        hnsw_from_snapshot
            .read_section(selene_graph::SubTag(sub_tag), &bytes)
            .expect("legacy HNSW section forwards to default");
    }
    for sub_tag in [*b"CQNT", *b"IPQB", *b"POST"] {
        let bytes = reader
            .read_section(*b"IVFP", sub_tag)
            .expect("legacy IVF section reads");
        ivf_from_snapshot
            .read_section(selene_graph::SubTag(sub_tag), &bytes)
            .expect("legacy IVF section forwards to default");
    }
    assert_eq!(
        hnsw_from_snapshot
            .get("default")
            .expect("default HNSW exists")
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("snapshot HNSW search succeeds")
            .first()
            .map(|(node_id, _)| *node_id),
        Some(NodeId::new(1))
    );
    assert_eq!(
        ivf_from_snapshot
            .get("default")
            .expect("default IVF exists")
            .snapshot()
            .len(),
        1
    );

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
    let fixture_dir = root.join(FIXTURE_DIR);
    fs::create_dir_all(&fixture_dir).expect("fixture dir exists");
    let _ = fs::remove_file(fixture_dir.join(SNAPSHOT_FILE));
    let _ = fs::remove_file(fixture_dir.join(format!("{SNAPSHOT_FILE}.tmp")));
    let _ = fs::remove_file(fixture_dir.join(WAL_FILE));
    write_snapshot_fixture(&fixture_dir);
    write_wal_fixture(&fixture_dir.join(WAL_FILE));
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
