//! Graph-layer error types and GQLSTATUS mappings.

use selene_core::{CoreError, EdgeId, NodeId};

/// Result alias for graph operations.
pub type GraphResult<T> = Result<T, GraphError>;

/// Error type for graph storage and mutation operations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum GraphError {
    /// The requested node row does not exist.
    #[error("node not found: {id}")]
    #[diagnostic(code(SLENE_G_001))]
    NodeNotFound {
        /// Missing node ID.
        id: NodeId,
    },

    /// The requested edge row does not exist.
    #[error("edge not found: {id}")]
    #[diagnostic(code(SLENE_G_002))]
    EdgeNotFound {
        /// Missing edge ID.
        id: EdgeId,
    },

    /// The requested node row exists but is not alive.
    #[error("node {id} is not alive")]
    #[diagnostic(code(SLENE_G_003))]
    NodeNotAlive {
        /// Dead node ID.
        id: NodeId,
    },

    /// The requested edge row exists but is not alive.
    #[error("edge {id} is not alive")]
    #[diagnostic(code(SLENE_G_004))]
    EdgeNotAlive {
        /// Dead edge ID.
        id: EdgeId,
    },

    /// Allocator advanced past the v1 row-addressable range (max 2^32 rows).
    #[error("{kind} id {raw} exceeds the v1 row-index range (max {max})")]
    #[diagnostic(code(SLENE_G_005))]
    IdOverflow {
        /// `"node"` or `"edge"`.
        kind: &'static str,
        /// The raw u64 ID that overflowed.
        raw: u64,
        /// The maximum addressable raw ID.
        max: u64,
    },

    /// Error propagated from selene-core.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(#[from] CoreError),
}

impl GraphError {
    /// Map this error to its 5-character ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> &'static str {
        match self {
            Self::NodeNotFound { .. }
            | Self::EdgeNotFound { .. }
            | Self::NodeNotAlive { .. }
            | Self::EdgeNotAlive { .. } => "22023",
            Self::IdOverflow { .. } => "53000",
            Self::Core(_) => "22000",
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(GraphError::NodeNotFound { id: NodeId::new(1) }, "22023")]
    #[case(GraphError::EdgeNotFound { id: EdgeId::new(1) }, "22023")]
    #[case(GraphError::NodeNotAlive { id: NodeId::new(1) }, "22023")]
    #[case(GraphError::EdgeNotAlive { id: EdgeId::new(1) }, "22023")]
    #[case(
        GraphError::IdOverflow { kind: "node", raw: 5_000_000_000, max: 4_294_967_296 },
        "53000"
    )]
    #[case(GraphError::Core(CoreError::ZeroIdentifier), "22000")]
    fn gqlstatus_for_each_variant(#[case] error: GraphError, #[case] status: &str) {
        assert_eq!(error.gqlstatus(), status);
    }

    #[test]
    fn core_error_variant_propagates() {
        fn inner() -> Result<(), CoreError> {
            Err(CoreError::ZeroIdentifier)
        }
        fn outer() -> GraphResult<()> {
            inner()?;
            Ok(())
        }
        assert!(matches!(
            outer(),
            Err(GraphError::Core(CoreError::ZeroIdentifier))
        ));
    }
}
