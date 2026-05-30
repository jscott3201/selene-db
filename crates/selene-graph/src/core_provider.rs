//! Core graph snapshot provider for persistence integration.

mod recovery_state;
mod sections;

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use selene_core::{Change, HlcTimestamp, Origin, SchemaChange};
use selene_persist::{
    AUDIT_KIND_PACK_LIFECYCLE, AuditLog, AuditRecord, RecoveryError, RecoveryProvider,
    RecoveryResult, WalWriter,
};

use crate::core_provider::recovery_state::RecoveryState;
use crate::core_provider::sections::{
    encode_composite_schemas, encode_edges, encode_graph_types, encode_meta, encode_nodes,
    encode_schemas,
};
use crate::durable_provider::DurableProvider;
use crate::error::GraphResult;
use crate::graph::SeleneGraph;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};

/// Core graph provider tag used in snapshot section tables.
pub const CORE_PROVIDER_TAG: [u8; 4] = *b"CORE";
/// Core metadata subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_META_SUB: [u8; 4] = *b"META";
/// Core graph-type subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_GTYP_SUB: [u8; 4] = *b"GTYP";
/// Core node-column subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_NODE_SUB: [u8; 4] = *b"NODE";
/// Core edge-column subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_EDGE_SUB: [u8; 4] = *b"EDGE";
/// Core schema subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_SCMA_SUB: [u8; 4] = *b"SCMA";
/// Core composite-property-index schema subsection tag under [`CORE_PROVIDER_TAG`].
pub const CORE_CPIX_SUB: [u8; 4] = *b"CPIX";

const CORE_SUB_TAGS: &[SubTag] = &[
    SubTag(CORE_GTYP_SUB),
    SubTag(CORE_META_SUB),
    SubTag(CORE_NODE_SUB),
    SubTag(CORE_EDGE_SUB),
    SubTag(CORE_SCMA_SUB),
    SubTag(CORE_CPIX_SUB),
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
    Live {
        snapshot: Arc<ArcSwap<SeleneGraph>>,
        durable: Option<DurableState>,
    },
    Recovery {
        state: RecoveryState,
    },
}

/// Durable WAL state owned by a live [`CoreProvider`].
///
/// The HLC counter is seeded from [`WalWriter::last_sequence`]. On a fresh WAL
/// this is zero and the first commit receives `HlcTimestamp::new(1, 0)`; after
/// reopening, the next timestamp advances past the recovered WAL sequence.
pub struct DurableState {
    writer: Mutex<WalWriter>,
    next_hlc: AtomicU64,
    audit: Option<Mutex<AuditLog>>,
}

impl DurableState {
    /// Construct durable state from an already-open WAL writer.
    #[must_use]
    pub fn new(writer: WalWriter) -> Self {
        let last_sequence = writer.last_sequence();
        Self {
            writer: Mutex::new(writer),
            next_hlc: AtomicU64::new(last_sequence),
            audit: None,
        }
    }

    /// Attach an audit log so pack-lifecycle events committed through this
    /// provider are mirrored to it (Item 7 / Seam D, D24).
    ///
    /// Mirroring is **WAL-first, audit-after**: the WAL append is the source of
    /// truth and gates the commit; the audit write runs only after it succeeds
    /// and is best-effort (a failure is logged, never failing the commit). The
    /// lifecycle event also remains in the WAL, so a failed mirror degrades to
    /// the pre-Item-7 WAL-only behavior rather than losing the event. Per the
    /// donor lesson "audit lag is recoverable, fiction is not," the audit can
    /// only lag the WAL, never lead it.
    #[must_use]
    pub fn with_audit_log(mut self, audit: AuditLog) -> Self {
        self.audit = Some(Mutex::new(audit));
        self
    }
}

/// Current wall-clock time as nanoseconds since the Unix epoch, saturating.
fn unix_nanos_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Mirror every pack-lifecycle event in `changes` to the audit log.
///
/// Best-effort: a serialization or append failure is logged and skipped, never
/// propagated, because the WAL already holds the authoritative copy (see
/// [`DurableState::with_audit_log`]). All events from one commit share a single
/// wall-clock stamp.
fn mirror_lifecycle_to_audit(audit: &Mutex<AuditLog>, changes: &[Change]) {
    let recorded_at_unix_nanos = unix_nanos_now();
    let mut log = audit.lock();
    for change in changes {
        let Change::SchemaChanged {
            change: SchemaChange::ProcedurePackLifecycle { event },
            ..
        } = change
        else {
            continue;
        };
        match postcard::to_allocvec(event) {
            Ok(payload) => {
                let record = AuditRecord {
                    recorded_at_unix_nanos,
                    kind: AUDIT_KIND_PACK_LIFECYCLE,
                    payload,
                };
                if let Err(error) = log.append(&record) {
                    tracing::error!(%error, "audit: failed to mirror pack lifecycle event");
                }
            }
            Err(error) => {
                tracing::error!(%error, "audit: failed to encode pack lifecycle event");
            }
        }
    }
}

impl CoreProvider {
    /// Construct a live provider bound to a shared graph snapshot pointer.
    #[must_use]
    pub fn new_for_live(snapshot: Arc<ArcSwap<SeleneGraph>>) -> Arc<Self> {
        Self::new_for_live_with_wal(snapshot, None)
    }

    /// Construct a live provider with optional commit-critical WAL state.
    #[must_use]
    pub fn new_for_live_with_wal(
        snapshot: Arc<ArcSwap<SeleneGraph>>,
        durable: Option<DurableState>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(CoreInner::Live { snapshot, durable }),
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
        expected_bound_type: Option<Arc<crate::graph_types::GraphTypeDef>>,
    ) -> GraphResult<SeleneGraph> {
        let mut inner = self.inner.lock();
        match &mut *inner {
            CoreInner::Live { .. } => {
                Err(inconsistent("finish_recovery called on live-mode CoreProvider").into())
            }
            CoreInner::Recovery { state } => {
                let state = std::mem::take(state);
                state.into_graph(expected_graph_id, expected_bound_type)
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
            CoreInner::Live { snapshot, .. } => {
                let graph = snapshot.load_full();
                match sub_tag.0 {
                    CORE_GTYP_SUB => encode_graph_types(&graph),
                    CORE_META_SUB => encode_meta(&graph.meta, graph.meta.generation),
                    CORE_NODE_SUB => encode_nodes(&graph),
                    CORE_EDGE_SUB => encode_edges(&graph),
                    CORE_SCMA_SUB => encode_schemas(&graph),
                    CORE_CPIX_SUB => encode_composite_schemas(&graph),
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

    /// Shared per-WAL-entry truncate-expansion buffer for recovery fan-out.
    ///
    /// Recovery subscriber wrappers clone this handle so they can read the
    /// per-row `NodeDeleted`/`EdgeDeleted` tombstones CORE stages while
    /// re-deriving truncated rows from the recovered store (BRIEF-150 / audit
    /// Item 11). Returns `None` for a live-mode provider, which never truncates
    /// through recovery.
    #[must_use]
    pub(crate) fn truncate_expansion_handle(&self) -> Option<Arc<Mutex<Vec<Change>>>> {
        let inner = self.inner.lock();
        match &*inner {
            CoreInner::Live { .. } => None,
            CoreInner::Recovery { state } => Some(state.truncate_expansion_handle()),
        }
    }

    /// Clear the per-entry truncate-expansion buffer before applying a WAL
    /// entry's changes, so staged tombstones reflect only that entry.
    fn reset_truncate_expansion(&self) {
        let inner = self.inner.lock();
        if let CoreInner::Recovery { state } = &*inner {
            state.reset_truncate_expansion();
        }
    }
}

impl IndexProvider for CoreProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

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

impl DurableProvider for CoreProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(CORE_PROVIDER_TAG)
    }

    fn next_timestamp(&self) -> HlcTimestamp {
        let inner = self.inner.lock();
        match &*inner {
            CoreInner::Live {
                durable: Some(durable),
                ..
            } => {
                let seconds = durable
                    .next_hlc
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                HlcTimestamp::new(seconds, 0)
            }
            CoreInner::Live { durable: None, .. } | CoreInner::Recovery { .. } => {
                HlcTimestamp::zero()
            }
        }
    }

    fn write_commit(
        &self,
        principal: Option<&[u8]>,
        changes: &[Change],
        timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        let mut inner = self.inner.lock();
        match &mut *inner {
            CoreInner::Live {
                durable: Some(durable),
                ..
            } => {
                let principal = principal.map(Arc::<[u8]>::from);
                // WAL-first: the append gates the commit (its error fails it).
                let sequence = {
                    let mut writer = durable.writer.lock();
                    writer
                        .append(timestamp, Origin::Local, principal, changes)
                        .map_err(durable_error)?
                };
                // Audit-after: best-effort mirror of pack-lifecycle events, only
                // reached once the WAL append committed.
                if let Some(audit) = &durable.audit {
                    mirror_lifecycle_to_audit(audit, changes);
                }
                Ok(sequence)
            }
            CoreInner::Live { durable: None, .. } => Ok(0),
            CoreInner::Recovery { .. } => Err(inconsistent(
                "write_commit called on recovery-mode CoreProvider",
            )),
        }
    }

    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        let mut inner = self.inner.lock();
        match &mut *inner {
            CoreInner::Live {
                durable: Some(durable),
                ..
            } => {
                let mut writer = durable.writer.lock();
                writer.flush().map_err(durable_error)?;
                Ok(Some(writer.last_sequence()))
            }
            CoreInner::Live { durable: None, .. } | CoreInner::Recovery { .. } => Ok(None),
        }
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

    fn on_changes(&self, changes: &[Change]) -> RecoveryResult<()> {
        // Reset the truncate-expansion buffer once per WAL entry so the
        // per-row tombstones downstream extension providers read reflect only this
        // entry's truncations (BRIEF-150 / audit Item 11). CORE runs first in
        // the tag-sorted registry, so this clear happens before any wrapper
        // reads the buffer.
        self.reset_truncate_expansion();
        for change in changes {
            self.on_change_inner(change).map_err(box_provider_error)?;
        }
        Ok(())
    }
}

pub(crate) fn invalid_payload(reason: impl Into<String>) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: reason.into(),
    }
}

fn durable_error(error: impl std::error::Error) -> ProviderError {
    ProviderError::SerializationFailed {
        reason: error.to_string(),
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
