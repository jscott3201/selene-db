//! Pathfinding error type. Lazy detection per spec 16 §E15.

use thiserror::Error;

use crate::error::AlgorithmAborted;
use selene_core::NodeId;

/// Errors raised by pathfinding algorithms during traversal.
///
/// All variants are detected **lazily** — only edges actually traversed (or
/// projections actually consulted) trigger an error. Up-front validation is
/// the caller's responsibility if required.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PathfindingError {
    /// A negative finite weight was encountered on a traversed edge.
    /// Dijkstra-family algorithms require non-negative weights; silent
    /// acceptance would yield incorrect shortest paths.
    ///
    /// Why: fields are named `source_node` / `target_node` (not `source`
    /// / `target`) because `thiserror` auto-applies `#[source]` to any
    /// field named `source`, which would try to treat `NodeId` as a
    /// nested `Error`.
    #[error("negative weight {weight} on edge {source_node:?} -> {target_node:?}")]
    NegativeWeight {
        /// Source node of the offending edge.
        source_node: NodeId,
        /// Target node of the offending edge.
        target_node: NodeId,
        /// The negative weight value as extracted by the projection.
        weight: f64,
    },

    /// A `NaN` weight was encountered on a traversed edge. NaN values are a
    /// programmer error and would break heap ordering; rejected eagerly.
    #[error("NaN weight on edge {source_node:?} -> {target_node:?}")]
    NaNWeight {
        /// Source node of the offending edge.
        source_node: NodeId,
        /// Target node of the offending edge.
        target_node: NodeId,
    },

    /// `apsp` rejected the call because the projection's node count exceeds
    /// the caller-supplied `max_nodes` limit. Callers fall back to per-source
    /// `sssp` calls when this fires.
    #[error(
        "projection has {nodes} nodes, exceeding caller-supplied limit {limit} — use sssp instead"
    )]
    TooLarge {
        /// Actual node count of the projection.
        nodes: usize,
        /// Caller-supplied upper bound for APSP.
        limit: usize,
    },

    /// Algorithm observed cooperative cancellation or deadline expiry.
    #[error(transparent)]
    Aborted {
        /// Cancellation cause.
        #[from]
        source: AlgorithmAborted,
    },
}
