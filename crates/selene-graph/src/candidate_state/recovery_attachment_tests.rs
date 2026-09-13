use super::*;

#[test]
fn recovery_callbacks_stage_invisibly_until_prepare_and_commit() {
    let (spec, name, doc, _, _) = current_spec();
    let shared = SharedGraph::new(GraphId::new(81_019));
    let first = {
        let mut txn = shared.begin_write();
        let first = txn
            .mutator()
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        first
    };
    let provider = MaintainedCandidateStateProvider::from_graph([spec], &shared.read()).unwrap();
    let second = {
        let mut txn = shared.begin_write();
        let second = txn
            .mutator()
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        second
    };

    assert!(matches!(
        provider.commit_recovery_attachment(),
        Err(ProviderError::Inconsistent { .. })
    ));
    provider.reserve_recovery_attachment().unwrap();
    provider
        .on_change(&Change::NodeCreated {
            id: second,
            labels: LabelSet::single(doc),
            properties: PropertyMap::new(),
        })
        .unwrap();
    provider
        .on_commit_applied(shared.read().meta.generation)
        .unwrap();
    assert_eq!(candidate_nodes(&provider, &name), vec![first]);
    assert!(matches!(
        provider.commit_recovery_attachment(),
        Err(ProviderError::Inconsistent { .. })
    ));

    provider
        .prepare_recovery_attachment(&shared.read())
        .unwrap();
    assert_eq!(candidate_nodes(&provider, &name), vec![first]);
    provider.commit_recovery_attachment().unwrap();
    assert_eq!(candidate_nodes(&provider, &name), vec![second]);
    assert_eq!(
        IndexProvider::node_candidate_set(&provider, &name, &shared.read())
            .unwrap()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![second]
    );
    provider.finalize_recovery_attachment();
    assert!(provider.runtime.lock().recovery.is_none());
    provider.reserve_recovery_attachment().unwrap();
    provider.abort_recovery_attachment();
}

#[test]
fn recovery_abort_retains_prior_live_candidate_state() {
    let (spec, name, doc, _, _) = current_spec();
    let shared = SharedGraph::new(GraphId::new(81_020));
    let first = {
        let mut txn = shared.begin_write();
        let first = txn
            .mutator()
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        first
    };
    let provider = MaintainedCandidateStateProvider::from_graph([spec], &shared.read()).unwrap();
    provider.reserve_recovery_attachment().unwrap();
    provider
        .on_change(&Change::NodeCreated {
            id: NodeId::new(first.get() + 1),
            labels: LabelSet::single(doc),
            properties: PropertyMap::new(),
        })
        .unwrap();

    provider.abort_recovery_attachment();
    provider.abort_recovery_attachment();

    assert_eq!(candidate_nodes(&provider, &name), vec![first]);
}

#[test]
fn recovery_promoted_abort_restores_prior_state_generation_and_typed_cache() {
    let (spec, name, doc, _, _) = current_spec();
    let shared = SharedGraph::new(GraphId::new(81_021));
    let first = {
        let mut txn = shared.begin_write();
        let first = txn
            .mutator()
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        first
    };
    let prior_snapshot = shared.read();
    let prior_generation = prior_snapshot.meta.generation;
    let provider = MaintainedCandidateStateProvider::from_graph([spec], &prior_snapshot).unwrap();
    let prior_typed = IndexProvider::node_candidate_set(&provider, &name, &prior_snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(prior_typed.iter().collect::<Vec<_>>(), vec![first]);

    let second = {
        let mut txn = shared.begin_write();
        let second = txn
            .mutator()
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        second
    };
    let recovered_snapshot = shared.read();
    provider.reserve_recovery_attachment().unwrap();
    provider
        .on_change(&Change::NodeCreated {
            id: second,
            labels: LabelSet::single(doc),
            properties: PropertyMap::new(),
        })
        .unwrap();
    provider
        .on_commit_applied(recovered_snapshot.meta.generation)
        .unwrap();
    provider
        .prepare_recovery_attachment(&recovered_snapshot)
        .unwrap();
    provider.commit_recovery_attachment().unwrap();
    assert_eq!(candidate_nodes(&provider, &name), vec![second]);

    provider.abort_recovery_attachment();

    assert_eq!(provider.generation(), prior_generation);
    assert_eq!(candidate_nodes(&provider, &name), vec![first]);
    let restored = IndexProvider::node_candidate_set(&provider, &name, &prior_snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(restored.iter().collect::<Vec<_>>(), vec![first]);
    assert!(restored.shares_physical_layout_with(&prior_snapshot));
    assert!(restored.shares_workspace_binding_with(&prior_snapshot));
    assert!(provider.runtime.lock().recovery.is_none());
}
