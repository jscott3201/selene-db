//! Shared-graph helpers for maintained candidate-state providers.

use selene_core::IStr;

use crate::{
    CANDIDATE_STATE_PROVIDER_TAG, ProviderError, ProviderTag, SharedGraph, VectorCandidateSet,
    VectorCandidateStateInfo,
};

impl SharedGraph {
    /// Look up a generation-checked maintained vector candidate set by name.
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
        name: &IStr,
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
