//! Shared-graph helpers for maintained candidate-state providers.

use selene_core::DbString;

use crate::{
    CANDIDATE_STATE_PROVIDER_TAG, CandidateSet, Node, ProviderError, ProviderTag, SeleneGraph,
    SharedGraph, VectorCandidateSet, VectorCandidateStateInfo,
};

impl SharedGraph {
    /// Bind maintained node candidates to a caller-pinned snapshot.
    ///
    /// Graph identity and private runtime ancestry are validated before the
    /// provider is consulted. Old snapshots from this runtime remain valid
    /// across compaction, while ordinary publication is rejected by the
    /// provider's generation watermark.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] for a foreign graph/runtime, stale provider
    /// generation, or any tombstone, absent, or dead provider node ID.
    pub fn node_candidate_set(
        &self,
        name: &DbString,
        pinned: &SeleneGraph,
    ) -> Result<Option<CandidateSet<Node>>, ProviderError> {
        if pinned.meta.graph_id != self.graph_id {
            return Err(inconsistent(format!(
                "pinned graph {} does not match shared graph {}",
                pinned.meta.graph_id, self.graph_id
            )));
        }
        if !pinned.runtime_lineage.same_as(&self.runtime_lineage) {
            return Err(inconsistent(
                "pinned graph does not belong to this shared runtime".to_owned(),
            ));
        }
        let Some(provider) = self.index_provider_by_tag(ProviderTag(CANDIDATE_STATE_PROVIDER_TAG))
        else {
            return Ok(None);
        };
        let Some(unbound) = provider.vector_candidate_set(name, pinned.meta.generation)? else {
            return Ok(None);
        };
        CandidateSet::bind_provider_ids(pinned, unbound.as_nodes().iter().copied())
            .map(Some)
            .map_err(|id| {
                inconsistent(format!(
                    "candidate-state provider returned invalid node {id}"
                ))
            })
    }

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

fn inconsistent(reason: String) -> ProviderError {
    ProviderError::Inconsistent { reason }
}
