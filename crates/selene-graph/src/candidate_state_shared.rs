//! Shared-graph helpers for maintained candidate-state providers.

use selene_core::DbString;

use crate::{
    CANDIDATE_STATE_PROVIDER_TAG, CandidateSet, Node, ProviderError, ProviderTag, SeleneGraph,
    SharedGraph, VectorCandidateSet, VectorCandidateStateInfo,
};

impl SharedGraph {
    /// Look up snapshot-bound maintained node candidates by name.
    ///
    /// `graph` must be caller-pinned from this shared graph; the returned set
    /// remains usable with it after publication. `Ok(None)` means no maintained
    /// provider is registered or the name is unknown.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when provider state is stale or inconsistent.
    pub fn node_candidate_set(
        &self,
        name: &DbString,
        graph: &SeleneGraph,
    ) -> Result<Option<CandidateSet<Node>>, ProviderError> {
        let Some(provider) = self.index_provider_by_tag(ProviderTag(CANDIDATE_STATE_PROVIDER_TAG))
        else {
            return Ok(None);
        };
        provider.node_candidate_set(name, graph)
    }

    /// Look up a generation-checked maintained vector candidate set by name.
    ///
    /// This stable-ID result is the temporary M04-PR02 Part 2 bridge. New
    /// lower-layer consumers use [`Self::node_candidate_set`].
    ///
    /// `Ok(None)` means no maintained candidate-state provider is registered or
    /// the provider has no set named `name`. The returned set is tied to the
    /// same immutable snapshot generation used for the lookup.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the candidate-state provider is present
    /// but cannot prove it has applied through the current graph generation.
    pub fn vector_candidate_set(
        &self,
        name: &DbString,
    ) -> Result<Option<VectorCandidateSet>, ProviderError> {
        let snapshot = self.read();
        let Some(provider) = self.index_provider_by_tag(ProviderTag(CANDIDATE_STATE_PROVIDER_TAG))
        else {
            return Ok(None);
        };
        provider.vector_candidate_set(name, snapshot.meta.generation)
    }

    /// Return generation-checked metadata for maintained vector candidate states.
    ///
    /// An empty vector means no maintained candidate-state provider is
    /// registered for this graph. The metadata is tied to the same immutable
    /// graph snapshot that supplies the generation value, so callers can use the
    /// returned names immediately with vector candidate-state scoring.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the candidate-state provider is present
    /// but cannot prove it has applied through the current graph generation.
    pub fn vector_candidate_state_infos(
        &self,
    ) -> Result<Vec<VectorCandidateStateInfo>, ProviderError> {
        let snapshot = self.read();
        let Some(provider) = self.index_provider_by_tag(ProviderTag(CANDIDATE_STATE_PROVIDER_TAG))
        else {
            return Ok(Vec::new());
        };
        provider.vector_candidate_state_infos(snapshot.meta.generation)
    }
}
