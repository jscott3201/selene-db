//! Graph-layer error types and GQLSTATUS mappings.

use selene_core::{CoreError, EdgeId, IStr, NodeId};
use selene_persist::PersistError;

use crate::index_provider::ProviderError;
use crate::type_validator::TypeViolation;
use crate::typed_index::TypedIndexKind;

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

    /// The graph snapshot violates a structural invariant (e.g., row count
    /// exceeds the addressable u32 range).
    #[error("graph snapshot is inconsistent: {reason}")]
    #[diagnostic(code(SLENE_G_006))]
    Inconsistent {
        /// Free-form description of the inconsistency.
        reason: String,
    },

    /// A property index already exists for this `(label, property)`.
    #[error("property index already exists for ({label}, {property})")]
    #[diagnostic(code(SLENE_G_007))]
    PropertyIndexAlreadyExists {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
    },

    /// The named property index does not exist.
    #[error("property index does not exist for ({label}, {property})")]
    #[diagnostic(code(SLENE_G_008))]
    PropertyIndexNotFound {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
    },

    /// A value cannot be admitted to the declared property index kind.
    #[error(
        "property index ({label}, {property}) expected {expected_kind:?} but observed {observed}"
    )]
    #[diagnostic(code(SLENE_G_009))]
    IndexValueRejected {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
        /// Registered index kind.
        expected_kind: TypedIndexKind,
        /// Observed value kind or `"NaN"`.
        observed: &'static str,
    },

    /// A closed graph mutation violates its bound graph type.
    #[error(transparent)]
    #[diagnostic(transparent)]
    TypeViolation(#[from] TypeViolation),

    /// A commit-critical durable provider rejected or failed a write.
    #[error("durable provider failed: {reason}")]
    #[diagnostic(code(SLENE_G_015))]
    Durable {
        /// Human-readable durable provider failure reason.
        reason: String,
    },

    /// Error propagated from selene-core.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(#[from] CoreError),

    /// Error propagated from an index provider.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Provider(#[from] ProviderError),

    /// Error propagated from persistence recovery.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Persist(#[from] PersistError),
}

impl GraphError {
    /// Map this error to its 5-character ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> &'static str {
        match self {
            Self::NodeNotFound { .. }
            | Self::EdgeNotFound { .. }
            | Self::NodeNotAlive { .. }
            | Self::EdgeNotAlive { .. } => "22G03",
            Self::IdOverflow { .. } => "53000",
            Self::Inconsistent { .. } => "5GQL0",
            Self::PropertyIndexAlreadyExists { .. }
            | Self::PropertyIndexNotFound { .. }
            | Self::IndexValueRejected { .. } => "22G03",
            Self::TypeViolation(_) => "22000",
            Self::Core(_) => "22000",
            Self::Durable { .. } => "5GQL0",
            Self::Provider(_) | Self::Persist(_) => "5GQL0",
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use selene_core::intern;

    use super::*;
    use crate::ProviderError;

    #[rstest]
    #[case(GraphError::NodeNotFound { id: NodeId::new(1) }, "22G03")]
    #[case(GraphError::EdgeNotFound { id: EdgeId::new(1) }, "22G03")]
    #[case(GraphError::NodeNotAlive { id: NodeId::new(1) }, "22G03")]
    #[case(GraphError::EdgeNotAlive { id: EdgeId::new(1) }, "22G03")]
    #[case(
        GraphError::IdOverflow { kind: "node", raw: 5_000_000_000, max: 4_294_967_296 },
        "53000"
    )]
    #[case(
        GraphError::Inconsistent { reason: "row index exceeds u32::MAX".to_owned() },
        "5GQL0"
    )]
    #[case(
        GraphError::PropertyIndexAlreadyExists {
            label: intern("err.label").unwrap(),
            property: intern("err.property").unwrap(),
        },
        "22G03"
    )]
    #[case(
        GraphError::PropertyIndexNotFound {
            label: intern("err.label.missing").unwrap(),
            property: intern("err.property.missing").unwrap(),
        },
        "22G03"
    )]
    #[case(
        GraphError::IndexValueRejected {
            label: intern("err.label.rejected").unwrap(),
            property: intern("err.property.rejected").unwrap(),
            expected_kind: TypedIndexKind::I64,
            observed: "String",
        },
        "22G03"
    )]
    #[case(
        GraphError::TypeViolation(TypeViolation::UnknownEdgeLabel {
            id: EdgeId::new(1),
            label: intern("err.edge.label").unwrap(),
        }),
        "22000"
    )]
    #[case(GraphError::Core(CoreError::ZeroIdentifier), "22000")]
    #[case(GraphError::Durable { reason: "wal unavailable".to_owned() }, "5GQL0")]
    #[case(
        GraphError::Provider(ProviderError::Inconsistent { reason: "duplicate provider tag VECT".to_owned() }),
        "5GQL0"
    )]
    #[case(GraphError::Persist(PersistError::MalformedSnapshotFilename), "5GQL0")]
    fn gqlstatus_for_each_variant(#[case] error: GraphError, #[case] status: &str) {
        assert_eq!(error.gqlstatus(), status);
        assert!(
            selene_core::gqlstatus_name(status).is_some(),
            "GQLSTATUS code {status} emitted by GraphError but not in ALL_GQLSTATUS_NAMES"
        );
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
