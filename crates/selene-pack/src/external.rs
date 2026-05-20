//! Public external procedure-pack registration surface.
//!
//! This module is intentionally separate from the platform built-in traits in
//! `builtin`. External crates register graph procedures at registry-build time
//! without gaining access to the platform-only built-in contract.

use std::sync::Arc;

use selene_gql::{GqlType, GraphContext, MutationContext, ProcedureError, ProcedureResult, Value};

/// External graph and mutation procedures split by registry tier.
pub(crate) type ExternalProcedureSet = (
    Vec<Arc<dyn ExternalGraphProcedure>>,
    Vec<Arc<dyn ExternalMutationProcedure>>,
);

/// Procedure-pack description accepted by [`crate::ProcedurePackRegistryBuilder`].
#[derive(Clone)]
pub struct ExternalProcedurePack {
    name: &'static str,
    graph_procedures: Vec<Arc<dyn ExternalGraphProcedure>>,
    mutation_procedures: Vec<Arc<dyn ExternalMutationProcedure>>,
}

impl std::fmt::Debug for ExternalProcedurePack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalProcedurePack")
            .field("name", &self.name)
            .field("graph_procedure_count", &self.graph_procedures.len())
            .field("mutation_procedure_count", &self.mutation_procedures.len())
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
            mutation_procedures: Vec::new(),
        }
    }

    /// Add static mutation-tier procedures to this external pack.
    #[must_use]
    pub fn with_mutation_procedures(
        mut self,
        mutation_procedures: Vec<Arc<dyn ExternalMutationProcedure>>,
    ) -> Self {
        self.mutation_procedures = mutation_procedures;
        self
    }

    /// Static pack name used for duplicate-pack detection.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) fn into_procedures(self) -> ExternalProcedureSet {
        (self.graph_procedures, self.mutation_procedures)
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
    /// Human-readable parameter description.
    pub description: &'static str,
    /// Documentation-only default value text.
    pub default_doc: Option<&'static str>,
}

impl ExternalParameter {
    /// Construct external parameter metadata.
    #[must_use]
    pub const fn new(name: &'static str, ty: GqlType, nullable: bool) -> Self {
        Self {
            name,
            ty,
            nullable,
            description: "",
            default_doc: None,
        }
    }

    /// Attach a human-readable parameter description.
    #[must_use]
    pub const fn with_description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Attach documentation-only default value text.
    #[must_use]
    pub const fn with_default_doc(mut self, default_doc: &'static str) -> Self {
        self.default_doc = Some(default_doc);
        self
    }
}

/// One external procedure output column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalOutputColumn {
    /// Output column name.
    pub name: &'static str,
    /// Output column type.
    pub ty: GqlType,
    /// Human-readable output-column description.
    pub description: &'static str,
}

impl ExternalOutputColumn {
    /// Construct external output-column metadata.
    #[must_use]
    pub const fn new(name: &'static str, ty: GqlType) -> Self {
        Self {
            name,
            ty,
            description: "",
        }
    }

    /// Attach a human-readable output-column description.
    #[must_use]
    pub const fn with_description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }
}

/// Metadata exposed by an external graph procedure.
pub trait ExternalProcedureMetadata: Send + Sync + 'static {
    /// Canonical multipart procedure name.
    fn name(&self) -> &'static [&'static str];

    /// Human-readable procedure summary.
    fn description(&self) -> &'static str {
        ""
    }

    /// Version where this procedure became available.
    fn since_version(&self) -> &'static str {
        "1.0.0"
    }

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

/// Graph-write mutation procedure implemented by an external pack.
pub trait ExternalMutationProcedure: ExternalProcedureMetadata {
    /// Execute with mutation-tier graph access.
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}
