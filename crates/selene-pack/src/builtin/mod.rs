//! Built-in procedure definitions.

pub(crate) mod create_index;
pub(crate) mod drop_index;
pub(crate) mod health;

use selene_gql::{
    GqlType, GraphContext, MutationContext, ProcedureError, ProcedureMutability, ProcedureResult,
    ProcedureTier, Value,
};

/// Static parameter metadata exposed by a built-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StaticParameter {
    /// Parameter name.
    pub(crate) name: &'static str,
    /// Parameter type.
    pub(crate) ty: GqlType,
    /// Whether NULL is accepted.
    pub(crate) nullable: bool,
}

/// Static output-column metadata exposed by a built-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StaticOutputColumn {
    /// Output column name.
    pub(crate) name: &'static str,
    /// Output column type.
    pub(crate) ty: GqlType,
}

/// Metadata shared by every built-in procedure.
pub(crate) trait BuiltInMetadata: Send + Sync + 'static {
    /// Canonical multipart procedure name.
    fn name(&self) -> &'static [&'static str];

    /// Declared execution tier.
    fn tier(&self) -> ProcedureTier;

    /// Declared mutability.
    fn mutability(&self) -> ProcedureMutability;

    /// Static parameter metadata.
    fn signature_static(&self) -> &'static [StaticParameter];

    /// Static output-column metadata.
    fn output_columns_static(&self) -> &'static [StaticOutputColumn];

    /// Stable content hash. BRIEF-48 replaces the zero placeholder with a
    /// manifest-derived hash.
    fn content_hash(&self) -> [u8; 32] {
        [0_u8; 32]
    }
}

/// Graph-tier executable built-in.
pub(crate) trait GraphProcedureBuiltIn: BuiltInMetadata {
    /// Execute with read-only graph access.
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}

/// Mutation-tier executable built-in.
pub(crate) trait MutationProcedureBuiltIn: BuiltInMetadata {
    /// Execute inside an existing write transaction.
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}

#[cfg(test)]
mod tests {
    use super::{BuiltInMetadata, health::SeleneHealth};

    #[test]
    fn builtin_content_hash_defaults_to_zero_until_manifest_hash_lands() {
        assert_eq!(SeleneHealth.content_hash(), [0_u8; 32]);
    }
}
