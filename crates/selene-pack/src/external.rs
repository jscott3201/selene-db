//! Public external procedure-pack registration surface.
//!
//! This module is intentionally separate from the platform built-in traits in
//! `builtin`. External crates register graph procedures at registry-build time
//! without gaining access to the platform-only built-in contract.

use std::sync::Arc;

use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};

/// Procedure-pack description accepted by [`crate::ProcedurePackRegistryBuilder`].
#[derive(Clone)]
pub struct ExternalProcedurePack {
    name: &'static str,
    graph_procedures: Vec<Arc<dyn ExternalGraphProcedure>>,
}

impl std::fmt::Debug for ExternalProcedurePack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalProcedurePack")
            .field("name", &self.name)
            .field("graph_procedure_count", &self.graph_procedures.len())
            .finish()
    }
}

impl ExternalProcedurePack {
    /// Create a static external procedure-pack description.
    #[must_use]
    pub fn new(name: &'static str, graph_procedures: Vec<Arc<dyn ExternalGraphProcedure>>) -> Self {
        Self {
            name,
            graph_procedures,
        }
    }

    /// Static pack name used for duplicate-pack detection.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) fn into_graph_procedures(self) -> Vec<Arc<dyn ExternalGraphProcedure>> {
        self.graph_procedures
    }
}

/// One external procedure parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalParameter {
    /// Parameter name.
    pub name: &'static str,
    /// Parameter type.
    pub ty: GqlType,
    /// Whether NULL is accepted.
    pub nullable: bool,
}

/// One external procedure output column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalOutputColumn {
    /// Output column name.
    pub name: &'static str,
    /// Output column type.
    pub ty: GqlType,
}

/// Metadata exposed by an external graph procedure.
pub trait ExternalProcedureMetadata: Send + Sync + 'static {
    /// Canonical multipart procedure name.
    fn name(&self) -> &'static [&'static str];

    /// Positional input signature.
    fn signature(&self) -> Vec<ExternalParameter>;

    /// Output columns.
    fn output_columns(&self) -> Vec<ExternalOutputColumn>;
}

/// Read-only graph-tier procedure implemented by an external pack.
pub trait ExternalGraphProcedure: ExternalProcedureMetadata {
    /// Execute with read-only graph access.
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}
