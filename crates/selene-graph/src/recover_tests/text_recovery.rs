use std::fs;

use selene_core::{Change, GraphId, LabelSet, NodeId, SchemaChange, Value, db_string};

use super::*;

fn hit_ids(
    graph: &SharedGraph,
    label: &selene_core::DbString,
    property: &selene_core::DbString,
    query: &str,
) -> Vec<NodeId> {
    graph
        .read()
        .text_index_for(label, property)
        .unwrap()
        .search(query, 10)
        .into_iter()
        .map(|hit| hit.node_id)
        .collect()
}

#[test]
fn recover_snapshot_preserves_text_index_registration() {
    let dir = temp_dir("snapshot-text-index");
    let label = db_string("recover.text.index.node").unwrap();
    let property = db_string("recover.text.index.body").unwrap();
    let shared = SharedGraph::builder(GraphId::new(45)).build().unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                prop(
                    "recover.text.index.body",
                    Value::String(db_string("alpha beta").unwrap()),
                ),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_text_index_named(
            label.clone(),
            property.clone(),
            Some(db_string("recover_text_idx").unwrap()),
        )
        .unwrap();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir, GraphId::new(45)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.text_index_for(&label, &property).unwrap();
    assert_eq!(index.stats().documents, 1);
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    assert_eq!(
        snapshot
            .iter_text_index_entries()
            .next()
            .and_then(|(_, _, _, _, name)| name),
        Some(db_string("recover_text_idx").unwrap())
    );
    drop(snapshot);
    assert_eq!(
        hit_ids(&recovered, &label, &property, "alpha"),
        vec![NodeId::new(1)]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_wal_only_replays_text_index_registration() {
    let dir = temp_dir("wal-text-index");
    let label = db_string("recover.wal.text.index.node").unwrap();
    let property = db_string("recover.wal.text.index.body").unwrap();
    append_wal(
        &dir,
        0,
        &[
            Change::NodeCreated {
                id: NodeId::new(1),
                labels: LabelSet::single(label.clone()),
                properties: prop(
                    "recover.wal.text.index.body",
                    Value::String(db_string("gamma delta").unwrap()),
                ),
            },
            Change::SchemaChanged {
                graph: GraphId::new(46),
                change: SchemaChange::TextIndexCreated {
                    label: label.clone(),
                    property: property.clone(),
                    name: None,
                },
            },
        ],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(46)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.text_index_for(&label, &property).unwrap();
    assert_eq!(index.stats().documents, 1);
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    drop(snapshot);
    assert_eq!(
        hit_ids(&recovered, &label, &property, "gamma"),
        vec![NodeId::new(1)]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_wal_only_replays_text_index_drop() {
    let dir = temp_dir("wal-text-index-drop");
    let label = db_string("recover.wal.text.drop.node").unwrap();
    let property = db_string("recover.wal.text.drop.body").unwrap();
    append_wal(
        &dir,
        0,
        &[
            Change::SchemaChanged {
                graph: GraphId::new(47),
                change: SchemaChange::TextIndexCreated {
                    label: label.clone(),
                    property: property.clone(),
                    name: Some(db_string("recover_wal_text_drop_idx").unwrap()),
                },
            },
            Change::SchemaChanged {
                graph: GraphId::new(47),
                change: SchemaChange::TextIndexDropped {
                    label: label.clone(),
                    property: property.clone(),
                },
            },
        ],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(47)).unwrap();
    assert!(recovered.read().text_index_for(&label, &property).is_none());
    let _ = fs::remove_dir_all(dir);
}
