//! Graph-commit-routed lifecycle sink.

use std::sync::Arc;

use selene_core::{GraphId, IStr, PackLifecycleEvent, SchemaChange, intern};
use selene_graph::SharedGraph;

use super::{ActivationError, LifecycleEvent, LifecycleSink};

/// `LifecycleSink` implementation that emits lifecycle audit through the graph
/// mutation funnel.
///
/// This type does not write `selene-persist` WAL files directly. Embedders that
/// need durable audit register a WAL-writing `IndexProvider`; this sink emits
/// `SchemaChange::ProcedurePackLifecycle` so that provider can observe and
/// persist the change.
pub struct GraphCommitSink {
    graph: Arc<SharedGraph>,
    graph_anchor: GraphId,
    principal_bytes: Option<Arc<[u8]>>,
}

impl GraphCommitSink {
    /// Construct a graph-commit lifecycle sink anchored to `graph_anchor`.
    ///
    /// # Panics
    ///
    /// Panics when `graph_anchor` disagrees with the supplied
    /// [`SharedGraph`]'s identity. Mismatched inputs would emit
    /// `Change::SchemaChanged { graph: graph_anchor, .. }` against a graph
    /// the sink does not actually mutate, corrupting per-graph audit
    /// attribution. The two ids must match at construction time.
    #[must_use]
    pub fn new(graph: Arc<SharedGraph>, graph_anchor: GraphId) -> Self {
        let actual = graph.read().graph_id();
        assert_eq!(
            actual, graph_anchor,
            "GraphCommitSink anchor mismatch: supplied {graph_anchor:?} but SharedGraph identity is {actual:?}",
        );
        Self {
            graph,
            graph_anchor,
            principal_bytes: None,
        }
    }

    /// Attach caller-owned principal bytes for the graph commit boundary.
    #[must_use]
    pub fn with_principal_bytes(mut self, principal_bytes: Arc<[u8]>) -> Self {
        self.principal_bytes = Some(principal_bytes);
        self
    }

    /// Return the graph anchor used for emitted schema changes.
    #[must_use]
    pub const fn graph_anchor(&self) -> GraphId {
        self.graph_anchor
    }

    /// Return the caller-owned principal bytes forwarded to graph commits.
    #[must_use]
    pub fn principal_bytes(&self) -> Option<&Arc<[u8]>> {
        self.principal_bytes.as_ref()
    }
}

impl LifecycleSink for GraphCommitSink {
    fn record(&self, event: &LifecycleEvent) -> Result<(), ActivationError> {
        let event = lifecycle_to_payload(event)?;
        let mut txn = self.graph.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator.schema_change(
                self.graph_anchor,
                SchemaChange::ProcedurePackLifecycle { event },
            );
        }
        txn.commit_with_principal(self.principal_bytes.clone())
            .map_err(|source| ActivationError::SinkRefused {
                detail: source.to_string(),
            })?;
        Ok(())
    }
}

fn lifecycle_to_payload(event: &LifecycleEvent) -> Result<PackLifecycleEvent, ActivationError> {
    lifecycle_to_payload_with(event, intern_for_payload)
}

fn lifecycle_to_payload_with(
    event: &LifecycleEvent,
    mut intern_value: impl FnMut(&str) -> Result<IStr, ActivationError>,
) -> Result<PackLifecycleEvent, ActivationError> {
    Ok(match event {
        LifecycleEvent::ValidationFailed {
            pack_name,
            principal,
            error,
            at,
        } => PackLifecycleEvent::ValidationFailed {
            pack_name: pack_name.as_deref().map(&mut intern_value).transpose()?,
            principal: principal.as_istr(),
            // `error` is free-form diagnostic text; do not admit it to the
            // global IStr interner (per Codex P2 — repeated invalid uploads
            // would otherwise drain interner capacity and break unrelated
            // lifecycle records with `SinkRefused`).
            error: error.clone(),
            at: *at,
        },
        LifecycleEvent::Staged {
            pack_name,
            content_hash,
            principal,
            at,
        } => PackLifecycleEvent::Staged {
            pack_name: intern_value(pack_name)?,
            content_hash: content_hash.as_bytes(),
            principal: principal.as_istr(),
            at: *at,
        },
        LifecycleEvent::Activated {
            pack_name,
            content_hash,
            principal,
            at,
        } => PackLifecycleEvent::Activated {
            pack_name: intern_value(pack_name)?,
            content_hash: content_hash.as_bytes(),
            principal: principal.as_istr(),
            at: *at,
        },
        LifecycleEvent::Deprecated {
            pack_name,
            reason,
            principal,
            at,
        } => PackLifecycleEvent::Deprecated {
            pack_name: intern_value(pack_name)?,
            reason: *reason,
            principal: principal.as_istr(),
            at: *at,
        },
        LifecycleEvent::Disabled {
            pack_name,
            principal,
            at,
        } => PackLifecycleEvent::Disabled {
            pack_name: intern_value(pack_name)?,
            principal: principal.as_istr(),
            at: *at,
        },
    })
}

fn intern_for_payload(value: &str) -> Result<IStr, ActivationError> {
    intern(value).map_err(|_| ActivationError::SinkRefused {
        detail: "interner cap exhausted".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use selene_core::intern;

    use super::*;
    use crate::{ContentHash, Principal};

    #[test]
    fn lifecycle_to_payload_intern_failure_surfaces_as_sink_refused() {
        let principal = Principal::new(intern("graph_commit_sink.principal").unwrap());
        let event = LifecycleEvent::Staged {
            pack_name: "graph_commit_sink.pack".to_owned(),
            content_hash: ContentHash::placeholder(),
            principal,
            at: Timestamp::new(1, 0).unwrap(),
        };

        let err = lifecycle_to_payload_with(&event, |_| {
            Err(ActivationError::SinkRefused {
                detail: "interner cap exhausted".to_owned(),
            })
        })
        .expect_err("injected interner failure must fail conversion");

        assert!(matches!(
            err,
            ActivationError::SinkRefused { detail } if detail == "interner cap exhausted"
        ));
    }
}
