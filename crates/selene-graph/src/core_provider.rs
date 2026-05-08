//! Core graph snapshot provider for persistence integration.

mod recovery_state;
mod sections;

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use selene_core::Change;
use selene_persist::{RecoveryError, RecoveryProvider, RecoveryResult};

use crate::core_provider::recovery_state::RecoveryState;
use crate::core_provider::sections::{encode_edges, encode_meta, encode_nodes, encode_schemas};
use crate::error::GraphResult;
use crate::graph::SeleneGraph;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};

/// Core graph provider tag used in snapshot section tables.
pub const CORE_PROVIDER_TAG: [u8; 4] = *b"CORE";
/// Core metadata subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_META_SUB: [u8; 4] = *b"META";
/// Core node-column subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_NODE_SUB: [u8; 4] = *b"NODE";
/// Core edge-column subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_EDGE_SUB: [u8; 4] = *b"EDGE";
/// Core schema subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_SCMA_SUB: [u8; 4] = *b"SCMA";

const CORE_SUB_TAGS: &[SubTag] = &[
    SubTag(CORE_META_SUB),
    SubTag(CORE_NODE_SUB),
    SubTag(CORE_EDGE_SUB),
    SubTag(CORE_SCMA_SUB),
];

/// Shared provider implementation for live snapshots and recovery replay.
///
/// Live instances hold the exact [`ArcSwap`] used by [`crate::SharedGraph`].
/// Recovery instances accumulate snapshot sections and WAL changes until
/// [`CoreProvider::finish_recovery`] materializes a [`SeleneGraph`].
pub struct CoreProvider {
    inner: Mutex<CoreInner>,
}

enum CoreInner {
    Live { snapshot: Arc<ArcSwap<SeleneGraph>> },
    Recovery { state: RecoveryState },
}

impl CoreProvider {
    /// Construct a live provider bound to a shared graph snapshot pointer.
    #[must_use]
    pub fn new_for_live(snapshot: Arc<ArcSwap<SeleneGraph>>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(CoreInner::Live { snapshot }),
        })
    }

    /// Construct a recovery-mode provider with an empty accumulator.
    #[must_use]
    pub fn new_for_recovery() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(CoreInner::Recovery {
                state: RecoveryState::new(),
            }),
        })
    }

    /// Drain the recovery accumulator into a graph snapshot.
    ///
    /// `expected_graph_id` is the caller-asserted graph identity. If a
    /// snapshot's `CORE/META` was applied and disagrees with this id,
    /// recovery fails. If no `CORE/META` was applied (WAL-only or empty
    /// recovery), `expected_graph_id` is used directly with default scalar
    /// metadata fields.
    ///
    /// # Errors
    ///
    /// Returns [`crate::GraphError::Provider`] if this provider was
    /// constructed for live mode, if META disagrees with
    /// `expected_graph_id`, or if the accumulated section/changelog state
    /// cannot be materialized into graph columns.
    pub fn finish_recovery(
        self: Arc<Self>,
        expected_graph_id: selene_core::GraphId,
    ) -> GraphResult<SeleneGraph> {
        let mut inner = self.inner.lock();
        match &mut *inner {
            CoreInner::Live { .. } => {
                Err(inconsistent("finish_recovery called on live-mode CoreProvider").into())
            }
            CoreInner::Recovery { state } => {
                let state = std::mem::take(state);
                state.into_graph(expected_graph_id)
            }
        }
    }

    fn read_section_inner(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        let mut inner = self.inner.lock();
        match &mut *inner {
            CoreInner::Live { .. } => Err(inconsistent(
                "read_section called on live-mode CoreProvider",
            )),
            CoreInner::Recovery { state } => state.read_section(sub_tag, bytes),
        }
    }

    fn write_section_inner(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        let inner = self.inner.lock();
        match &*inner {
            CoreInner::Live { snapshot } => {
                let graph = snapshot.load_full();
                match sub_tag.0 {
                    CORE_META_SUB => encode_meta(&graph.meta, graph.meta.generation),
                    CORE_NODE_SUB => encode_nodes(&graph),
                    CORE_EDGE_SUB => encode_edges(&graph),
                    CORE_SCMA_SUB => encode_schemas(&graph),
                    _ => Err(invalid_sub_tag(sub_tag)),
                }
            }
            CoreInner::Recovery { .. } => Err(inconsistent(
                "write_section called on recovery-mode CoreProvider",
            )),
        }
    }

    fn on_change_inner(&self, change: &Change) -> Result<(), ProviderError> {
        let mut inner = self.inner.lock();
        match &mut *inner {
            CoreInner::Live { .. } => Ok(()),
            CoreInner::Recovery { state } => state.apply_change(change),
        }
    }
}

impl IndexProvider for CoreProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(CORE_PROVIDER_TAG)
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        self.read_section_inner(sub_tag, bytes)
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        self.write_section_inner(sub_tag)
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.on_change_inner(change)
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        CORE_SUB_TAGS
    }
}

impl RecoveryProvider for CoreProvider {
    fn provider_tag(&self) -> [u8; 4] {
        CORE_PROVIDER_TAG
    }

    fn read_section(&self, sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()> {
        self.read_section_inner(SubTag(sub), bytes)
            .map_err(box_provider_error)
    }

    fn on_change(&self, change: &Change) -> RecoveryResult<()> {
        self.on_change_inner(change).map_err(box_provider_error)
    }
}

pub(crate) fn invalid_payload(reason: impl Into<String>) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: reason.into(),
    }
}

pub(crate) fn serialization_failed(reason: impl Into<String>) -> ProviderError {
    ProviderError::SerializationFailed {
        reason: reason.into(),
    }
}

pub(crate) fn inconsistent(reason: impl Into<String>) -> ProviderError {
    ProviderError::Inconsistent {
        reason: reason.into(),
    }
}

fn invalid_sub_tag(sub_tag: SubTag) -> ProviderError {
    invalid_payload(format!("unknown CORE sub-tag {sub_tag}"))
}

fn box_provider_error(error: ProviderError) -> RecoveryError {
    Box::new(error)
}

#[cfg(test)]
#[path = "core_provider/tests.rs"]
mod tests;
