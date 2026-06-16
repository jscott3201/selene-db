use super::*;

#[test]
fn new_initial_state_is_empty() {
    let shared = SharedGraph::new(GraphId::new(1));
    assert_eq!(shared.read().node_count(), 0);
    assert!(
        shared
            .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
            .is_some()
    );
}
