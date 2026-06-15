//! Shared graph construction helpers.

use std::path::Path;
use std::sync::Arc;

use selene_core::GraphId;
use selene_persist::{AuditLog, SyncPolicy, WalConfig, WalWriter};

use super::SharedGraph;
use crate::committer_batch::CommitBatching;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::graph_types::GraphTypeDef;
use crate::index_provider::IndexProvider;

/// Builder for a [`SharedGraph`] and its fixed provider registry.
pub struct SharedGraphBuilder {
    graph: SeleneGraph,
    providers: Vec<Arc<dyn IndexProvider>>,
    wal_writer: Option<WalWriter>,
    audit_log: Option<AuditLog>,
    commit_batching: CommitBatching,
}

impl SharedGraphBuilder {
    /// Construct a builder for an empty graph.
    pub(super) fn new(graph_id: GraphId) -> Self {
        Self {
            graph: SeleneGraph::new(graph_id),
            providers: Vec::new(),
            wal_writer: None,
            audit_log: None,
            commit_batching: CommitBatching::Off,
        }
    }

    /// Register an index provider.
    ///
    /// Providers are retained in registration order, which is the order used
    /// for committed mutation delivery.
    #[must_use]
    pub fn with_provider(mut self, provider: Arc<dyn IndexProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Open a WAL file and route commits through the CORE durable provider.
    ///
    /// The path is the WAL file path, not a directory. Callers using the
    /// conventional layout should pass `dir.join(selene_persist::DEFAULT_WAL_FILE_NAME)`.
    ///
    /// # SyncPolicy is OVERRIDDEN (v1.2 BRIEF 2 — read this)
    ///
    /// The single per-graph committer thread is the **sole fsync caller** for the
    /// committer-managed WAL: it appends a contiguous run of commits with fsync
    /// deferred, then issues exactly one [`WalWriter::flush`] per run (the R1
    /// fsync-before-publish barrier). To make that the *only* fsync path, this
    /// method **forces `config.sync_policy` to [`SyncPolicy::OnFlushOnly`]**
    /// before opening the WAL — **whatever policy you pass is discarded.** The
    /// fsync cadence is instead controlled by [`Self::with_commit_batching`]:
    /// [`CommitBatching::Off`] (the default) fsyncs once per commit (behaviorally
    /// identical to the old `EveryN(1)`), and [`CommitBatching::On`] coalesces a
    /// contiguous run into one fsync. `config.snapshot_seq` is passed through
    /// verbatim. Durability is unchanged: the committer always flushes before it
    /// publishes or acks, so a commit is durable before it is ever visible.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the WAL cannot be opened, including
    /// when another writer already holds the file lock.
    pub fn with_wal(mut self, path: impl AsRef<Path>, mut config: WalConfig) -> GraphResult<Self> {
        // BRIEF 2: the committer owns fsync. Force OnFlushOnly before opening so
        // the committer's group flush is the single durability barrier. Done
        // before WalWriter::open so open-error timing (e.g. WriterLockHeld) is
        // unchanged for existing .unwrap() call sites.
        config.sync_policy = SyncPolicy::OnFlushOnly;
        self.wal_writer = Some(WalWriter::open(path.as_ref(), config)?);
        Ok(self)
    }

    /// Set the group-commit batching policy for the committer-managed WAL
    /// (v1.2 BRIEF 2). Default [`CommitBatching::Off`].
    ///
    /// With [`CommitBatching::Off`] the committer fsyncs once per commit
    /// (behaviorally identical to BRIEF 1). With [`CommitBatching::On`] it
    /// coalesces up to `max_commits` (capped by aggregate `max_bytes`) contiguous
    /// commits into one fsync — higher throughput + lower tail latency under
    /// fan-in, at the cost of grouping several commits behind one barrier (all
    /// still durable before any of them is acked or published). Has no effect
    /// without [`Self::with_wal`] (no durable provider to flush).
    #[must_use]
    pub fn with_commit_batching(mut self, batching: CommitBatching) -> Self {
        self.commit_batching = batching;
        self
    }

    /// Attach a durable audit log at `path` (conventionally
    /// `dir.join(selene_persist::DEFAULT_AUDIT_FILE_NAME)`).
    ///
    /// Engine-owned audit events committed through this graph are mirrored to
    /// the audit log so they survive WAL-archive pruning (Item 7 / Seam D, D24).
    /// Requires [`Self::with_wal`]: audit mirroring is part of the durable
    /// commit path, so [`Self::build`] errors if an audit log is configured
    /// without a WAL.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the audit log cannot be opened.
    pub fn with_audit_log(mut self, path: impl AsRef<Path>) -> GraphResult<Self> {
        self.audit_log = Some(AuditLog::open(path.as_ref()).map_err(GraphError::Persist)?);
        Ok(self)
    }

    /// Bind this graph to `type_def` at construction time.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the builder is already bound
    /// or when `type_def` fails self-consistency validation.
    pub fn bound_to(mut self, type_def: GraphTypeDef) -> GraphResult<Self> {
        if self.graph.meta.bound_type.is_some() {
            return Err(GraphError::Inconsistent {
                reason: "graph builder is already bound to a graph type".to_owned(),
            });
        }
        self.graph.meta.bound_type = Some(Arc::new(type_def.validate()?));
        Ok(self)
    }

    /// Build shared graph state and validate provider registration.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Provider`] when provider tags are duplicated.
    pub fn build(self) -> GraphResult<SharedGraph> {
        SharedGraph::from_graph_with_core_and_durables(
            self.graph,
            self.providers,
            Vec::new(),
            self.wal_writer,
            self.audit_log,
            self.commit_batching,
        )
    }
}
