//! Stateful provider protocol for engine-owned derived state.
//!
//! `IndexProvider` is the engine-internal hook through which the CORE storage
//! provider, maintained candidate-state providers, and recovery-side replay
//! providers participate in snapshot bootstrap and per-commit mutation
//! observation. selene-db is a single native engine with no loadable
//! extensions; providers are first-party engine internals, not loadable packs.

use std::fmt;

use selene_core::{Change, DbString};

use crate::{SeleneGraph, VectorCandidateSet};

/// Stable 4-byte ASCII identifier for an [`IndexProvider`] registration.
///
/// Reserved tag space per spec 04 section 4.2:
/// - `CORE` is reserved for engine-owned snapshot sections.
/// - `META`/`NODE`/`EDGE`/`SCMA` are reserved sub-tags under `CORE`, not
///   provider tags.
/// - First-party provider allocations include `CSET`, `TIMS`, `GRPR`.
/// - Other ASCII uppercase 4-byte sequences are provider-allocated.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProviderTag(
    /// Raw 4-byte provider tag.
    pub [u8; 4],
);

impl fmt::Display for ProviderTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_tag(self.0, f)
    }
}

/// 4-byte subsection identifier within a provider's snapshot footprint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubTag(
    /// Raw 4-byte provider-local subsection tag.
    pub [u8; 4],
);

impl fmt::Display for SubTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_tag(self.0, f)
    }
}

/// Stateful hook for engine-owned derived-state participation.
///
/// [`IndexProvider::read_section`] and [`IndexProvider::on_change`] take
/// `&self`: `selene-graph` stores providers as `Arc<dyn IndexProvider>`, so
/// providers use interior mutability for owned state. The engine guarantees
/// serialized calls per graph.
///
/// ## Change-shape contract
///
/// Runtime commit fan-out observes the post-commit graph snapshot. When a
/// mutation stages per-row tombstone expansions for
/// [`Change::NodesOfTypeTruncated`], [`Change::EdgesOfTypeTruncated`], or
/// [`Change::GraphReset`], live fan-out receives the expanded
/// `NodeDeleted`/`EdgeDeleted` view instead of the persisted declarative change.
/// WAL replay is different: recovery drives providers with the exact committed
/// `Change` payloads after CORE applies them. Providers whose derived state
/// depends on deleted rows must therefore handle the declarative truncate/reset
/// variants or rebuild their state from the recovered graph before serving
/// reads.
///
/// ## Re-entrancy contract
///
/// `on_change` MUST NOT initiate a write transaction on the same graph,
/// directly or indirectly. The engine detects same-thread re-entry into
/// `SharedGraph::begin_write` and panics with a clear message; the panic
/// is caught by the outer `notify_providers` boundary so the outer commit
/// still completes.
///
/// **Cross-thread re-entry is documented misuse.** A provider whose
/// `on_change` spawns a worker thread, calls `begin_write` on that worker,
/// and waits for the worker (e.g., `JoinHandle::join`, channel `recv`) is a
/// circular wait the engine cannot detect: the worker blocks on the held
/// write lock; the outer `on_change` blocks waiting for the worker; the
/// outer commit cannot release the lock until `on_change` returns.
/// Provider authors who spawn threads inside `on_change` must not block
/// the callback on those threads' progress.
pub trait IndexProvider: Send + Sync + 'static {
    /// Stable 4-byte ASCII tag uniquely identifying this provider.
    fn provider_tag(&self) -> ProviderTag;

    /// Snapshot bootstrap for one provider-owned section.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the payload is missing, malformed, or
    /// inconsistent with provider state.
    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError>;

    /// Snapshot publish for one provider-owned section.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when serialization cannot produce a stable
    /// section payload.
    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError>;

    /// Observe one committed mutation.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] for provider-local failures. Live commits log
    /// and continue after these errors because the graph snapshot has already
    /// been published.
    fn on_change(&self, change: &Change) -> Result<(), ProviderError>;

    /// Return true when this provider wants one callback per committed change batch.
    ///
    /// The default is `false`, preserving per-change panic/error isolation for
    /// simple observers. Providers that maintain state whose invariants span
    /// several changes in one commit can opt in and override [`Self::on_changes`]
    /// to apply the batch under one internal lock.
    fn handles_change_batches(&self) -> bool {
        false
    }

    /// Observe all changes from one committed mutation batch.
    ///
    /// The default delegates to [`Self::on_change`] in order. Live fanout calls
    /// this method only when [`Self::handles_change_batches`] returns true;
    /// recovery may call it for all providers because no concurrent readers can
    /// observe recovery-side intermediate state.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] for provider-local failures.
    fn on_changes(&self, changes: &[Change]) -> Result<(), ProviderError> {
        for change in changes {
            self.on_change(change)?;
        }
        Ok(())
    }

    /// Rebuild provider-owned derived state from a recovered graph snapshot.
    ///
    /// Recovery calls this when a provider declares persisted snapshot sections,
    /// a graph snapshot was applied, and that snapshot did not contain this
    /// provider's section. Providers that can derive their complete state from
    /// CORE graph rows should override this method.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the provider cannot rebuild safely.
    fn rebuild_from_graph(&self, _graph: &SeleneGraph) -> Result<(), ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: format!(
                "provider {} has persisted sections but does not support graph rebuild",
                self.provider_tag()
            ),
        })
    }

    /// Observe that every change for a committed graph generation was applied.
    ///
    /// Live fan-out calls this only after the provider's mutation callback path
    /// completed without returning an error or panicking. Providers that expose
    /// read-side state tied to graph generations can use this as their
    /// visibility watermark.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] for provider-local failures.
    fn on_commit_applied(&self, _generation: u64) -> Result<(), ProviderError> {
        Ok(())
    }

    /// Return a provider-owned vector candidate set for `name` at `generation`.
    ///
    /// Providers that do not own named vector candidate state return `Ok(None)`.
    /// A provider that owns the name but cannot prove the requested generation
    /// should return an error instead of serving stale derived state.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the named state exists but is stale or
    /// internally inconsistent.
    fn vector_candidate_set(
        &self,
        _name: &DbString,
        _generation: u64,
    ) -> Result<Option<VectorCandidateSet>, ProviderError> {
        Ok(None)
    }

    /// Return provider-owned vector candidate-state descriptors at `generation`.
    ///
    /// Providers that do not own named vector candidate state return an empty
    /// vector. A provider that owns candidate state but cannot prove the
    /// requested generation should return an error instead of exposing stale
    /// metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when provider metadata is stale or internally
    /// inconsistent.
    fn vector_candidate_state_infos(
        &self,
        _generation: u64,
    ) -> Result<Vec<VectorCandidateStateInfo>, ProviderError> {
        Ok(Vec::new())
    }

    /// Provider-owned snapshot subsection tags.
    ///
    /// Empty means the provider consumes mutation events but owns no persisted
    /// snapshot state.
    fn declared_sub_tags(&self) -> &[SubTag];
}

/// Metadata for one provider-owned vector candidate state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorCandidateStateInfo {
    /// Stable set name used by callers to retrieve candidates.
    pub name: DbString,
    /// Graph generation the descriptor was derived from.
    pub generation: u64,
    /// Number of nodes currently in the candidate set.
    pub candidate_count: usize,
    /// Optional node label required for membership.
    pub required_label: Option<DbString>,
    /// Outgoing edge labels required on the source node.
    pub require_outgoing: Vec<DbString>,
    /// Incoming edge labels required on the target node.
    pub require_incoming: Vec<DbString>,
    /// Outgoing edge labels that disqualify the source node.
    pub exclude_outgoing: Vec<DbString>,
    /// Incoming edge labels that disqualify the target node.
    pub exclude_incoming: Vec<DbString>,
}

/// Errors returned by [`IndexProvider`] implementations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ProviderError {
    /// Provider payload could not be decoded or validated.
    #[error("invalid provider payload: {reason}")]
    #[diagnostic(code(SLENE_G_010))]
    InvalidPayload {
        /// Human-readable provider failure reason.
        reason: String,
    },

    /// Provider state could not be serialized.
    #[error("provider serialization failed: {reason}")]
    #[diagnostic(code(SLENE_G_012))]
    SerializationFailed {
        /// Human-readable serialization failure reason.
        reason: String,
    },

    /// Provider state or registration is inconsistent.
    #[error("provider state inconsistency: {reason}")]
    #[diagnostic(code(SLENE_G_014))]
    Inconsistent {
        /// Human-readable inconsistency reason.
        reason: String,
    },
}

fn fmt_tag(bytes: [u8; 4], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if bytes.iter().all(|byte| byte.is_ascii_graphic()) {
        for byte in bytes {
            f.write_str(char::from(byte).encode_utf8(&mut [0; 4]))?;
        }
        Ok(())
    } else {
        write!(
            f,
            "0x{:02X}{:02X}{:02X}{:02X}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;
    use rstest::rstest;
    use selene_core::{LabelSet, NodeId, PropertyMap};

    use super::*;
    use crate::{GraphError, GraphResult};

    struct RecordingProvider {
        tag: ProviderTag,
        changes: Mutex<Vec<Change>>,
    }

    impl RecordingProvider {
        fn new(tag: ProviderTag) -> Self {
            Self {
                tag,
                changes: Mutex::new(Vec::new()),
            }
        }
    }

    impl IndexProvider for RecordingProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
            self.changes.lock().push(change.clone());
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    fn assert_send_sync_static<T: Send + Sync + 'static>() {}

    #[test]
    fn provider_tag_equality_and_ordering() {
        let demo = ProviderTag(*b"DEMO");
        let meta = ProviderTag(*b"META");
        assert_eq!(demo, ProviderTag(*b"DEMO"));
        assert!(demo < meta);
        assert_eq!(demo.to_string(), "DEMO");
    }

    #[test]
    fn sub_tag_equality_and_ordering() {
        let graph = SubTag(*b"GRPH");
        let subt = SubTag(*b"SUBT");
        assert_eq!(graph, SubTag(*b"GRPH"));
        assert!(graph < subt);
        assert_eq!(graph.to_string(), "GRPH");
    }

    #[rstest]
    #[case(ProviderError::InvalidPayload { reason: "bad".to_owned() })]
    #[case(ProviderError::SerializationFailed { reason: "io".to_owned() })]
    #[case(ProviderError::Inconsistent { reason: "duplicate".to_owned() })]
    fn provider_error_gqlstatus_mappings(#[case] provider_error: ProviderError) {
        let graph_error = GraphError::Provider(provider_error);
        assert_eq!(graph_error.gqlstatus(), "5GQL0");
    }

    #[test]
    fn dummy_provider_with_interior_mutability() -> GraphResult<()> {
        assert_send_sync_static::<RecordingProvider>();
        let provider = RecordingProvider::new(ProviderTag(*b"TEST"));
        provider.on_change(&Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::new(),
            properties: PropertyMap::new(),
        })?;
        assert_eq!(provider.changes.lock().len(), 1);
        assert_eq!(provider.provider_tag(), ProviderTag(*b"TEST"));
        Ok(())
    }
}
