use std::sync::Arc;

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, db_string};

use super::super::*;
use crate::SharedGraph;

fn label(name: &str) -> DbString {
    db_string(name).unwrap()
}

fn candidate_nodes(provider: &MaintainedCandidateStateProvider, name: &DbString) -> Vec<NodeId> {
    provider
        .candidate_set(name)
        .expect("candidate set is configured")
        .into_nodes()
}

#[test]
fn provider_tracks_required_edge_constraints_through_commits() {
    let name = label("evidence-backed");
    let doc = label("MemoryFact");
    let supports = label("SUPPORTS");
    let cited_by = label("CITED_BY");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([CandidateStateSpec::new(name.clone())
            .require_label(doc.clone())
            .require_outgoing(supports.clone())
            .require_incoming(cited_by.clone())])
        .unwrap(),
    );
    let shared = SharedGraph::builder(GraphId::new(81_018))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();

    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let candidate = mutator
        .create_node(LabelSet::single(doc), PropertyMap::new())
        .unwrap();
    let target = mutator
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    let citation_source = mutator
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    assert!(candidate_nodes(&provider, &name).is_empty());

    let mut txn = shared.begin_write();
    let outgoing = txn
        .mutator()
        .create_edge(supports, candidate, target, PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    assert!(candidate_nodes(&provider, &name).is_empty());

    let mut txn = shared.begin_write();
    let incoming = txn
        .mutator()
        .create_edge(cited_by, citation_source, candidate, PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    assert_eq!(candidate_nodes(&provider, &name), vec![candidate]);

    let mut txn = shared.begin_write();
    txn.mutator().delete_edge(outgoing).unwrap();
    txn.commit().unwrap();
    assert!(candidate_nodes(&provider, &name).is_empty());

    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    mutator
        .create_edge(label("SUPPORTS"), candidate, target, PropertyMap::new())
        .unwrap();
    mutator.delete_edge(incoming).unwrap();
    txn.commit().unwrap();
    assert!(candidate_nodes(&provider, &name).is_empty());

    let mut txn = shared.begin_write();
    txn.mutator()
        .create_edge(
            label("CITED_BY"),
            citation_source,
            candidate,
            PropertyMap::new(),
        )
        .unwrap();
    txn.commit().unwrap();
    assert_eq!(candidate_nodes(&provider, &name), vec![candidate]);
}
