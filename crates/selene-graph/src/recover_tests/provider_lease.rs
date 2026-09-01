use std::sync::Arc;

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, db_string};

use super::{temp_dir, write_snapshot};
use crate::{
    CandidateStateSpec, GraphError, IndexProvider, MaintainedCandidateStateProvider, ProviderError,
    SharedGraph,
};

fn label(value: &str) -> DbString {
    db_string(value).unwrap()
}

fn provider(spec: CandidateStateSpec) -> Arc<MaintainedCandidateStateProvider> {
    Arc::new(MaintainedCandidateStateProvider::new([spec]).unwrap())
}

fn add_candidates(shared: &SharedGraph, node_label: &DbString, count: usize) -> Vec<NodeId> {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let ids = (0..count)
        .map(|_| {
            mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap()
        })
        .collect();
    txn.commit().unwrap();
    ids
}

fn assert_live_runtime_rejection(result: Result<SharedGraph, GraphError>) {
    match result {
        Err(GraphError::Provider(ProviderError::Inconsistent { reason })) => {
            assert!(reason.contains("already attached to a live runtime"));
        }
        Err(error) => panic!("expected live-runtime provider rejection, got {error}"),
        Ok(_) => panic!("a live runtime must retain provider exclusivity"),
    }
}

#[test]
fn recovery_reuse_rejects_before_mutating_live_candidate_state() {
    let graph_id = GraphId::new(82_100);
    let state_name = label("active");
    let document = label("Document");
    let spec = CandidateStateSpec::new(state_name.clone()).require_label(document.clone());
    let dir = temp_dir("candidate-provider-live-lease");

    let source_provider = provider(spec.clone());
    let source = SharedGraph::builder(graph_id)
        .with_provider(source_provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let source_ids = add_candidates(&source, &document, 2);
    source.begin_write().commit().unwrap();
    assert_eq!(source_provider.generation(), 2);
    write_snapshot(&dir, &source, 1);
    drop(source);
    drop(source_provider);

    let live_provider = provider(spec);
    let live = SharedGraph::builder(graph_id)
        .with_provider(live_provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let live_id = add_candidates(&live, &document, 1)[0];
    let pinned = live.read();
    assert_eq!(pinned.meta.generation, 1);
    assert_eq!(live_provider.generation(), 1);

    assert_live_runtime_rejection(SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![live_provider.clone() as Arc<dyn IndexProvider>],
    ));

    assert_eq!(pinned.meta.generation, 1);
    assert_eq!(live_provider.generation(), 1);
    assert_eq!(
        live.node_candidate_set(&state_name, &pinned)
            .unwrap()
            .unwrap()
            .iter_ids(&pinned)
            .unwrap()
            .collect::<Vec<_>>(),
        [live_id]
    );

    drop(live);
    assert_live_runtime_rejection(SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![live_provider.clone() as Arc<dyn IndexProvider>],
    ));
    assert_eq!(live_provider.generation(), 1);

    drop(pinned);
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![live_provider.clone() as Arc<dyn IndexProvider>],
    )
    .unwrap();
    let recovered_snapshot = recovered.read();
    assert_eq!(recovered_snapshot.meta.generation, 2);
    assert_eq!(live_provider.generation(), 2);
    assert_eq!(
        recovered
            .node_candidate_set(&state_name, &recovered_snapshot)
            .unwrap()
            .unwrap()
            .iter_ids(&recovered_snapshot)
            .unwrap()
            .collect::<Vec<_>>(),
        source_ids
    );

    drop(recovered_snapshot);
    drop(recovered);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_provider_recovery_releases_runtime_reservation() {
    let graph_id = GraphId::new(82_101);
    let source_name = label("source");
    let document = label("Document");
    let source_spec = CandidateStateSpec::new(source_name).require_label(document.clone());
    let dir = temp_dir("candidate-provider-reservation-release");

    let source_provider = provider(source_spec);
    let source = SharedGraph::builder(graph_id)
        .with_provider(source_provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    add_candidates(&source, &document, 1);
    write_snapshot(&dir, &source, 1);
    drop(source);
    drop(source_provider);

    let drifted_provider = provider(CandidateStateSpec::new(label("drifted")));
    let error = match SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![drifted_provider.clone() as Arc<dyn IndexProvider>],
    ) {
        Err(error) => error,
        Ok(_) => panic!("provider snapshot drift must fail recovery"),
    };
    assert!(error.to_string().contains("specs differ"));

    let replacement = SharedGraph::builder(GraphId::new(82_102))
        .with_provider(drifted_provider as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    drop(replacement);
    std::fs::remove_dir_all(dir).unwrap();
}
