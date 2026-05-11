//! Built-in procedure definitions.

pub(crate) mod create_index;
pub(crate) mod drop_index;
pub(crate) mod health;
pub(crate) mod pack_history;

use selene_gql::{
    GqlType, GraphContext, MutationContext, ProcedureError, ProcedureMutability, ProcedureResult,
    ProcedureTier, Value,
};

/// Local sentinel for built-in procedures that do not carry user-pack manifests.
///
/// This is distinct from the deleted user-pack content-hash placeholder.
/// Future work may assign deterministic hashes derived from built-in code
/// identity; until then, registry duplicate detection must not depend on
/// built-in hash differentiation.
pub(crate) const UNSTABLE_BUILTIN_CONTENT_HASH: [u8; 32] = [0_u8; 32];

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

    /// Stable content hash for registry duplicate detection.
    fn content_hash(&self) -> [u8; 32] {
        UNSTABLE_BUILTIN_CONTENT_HASH
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
    use std::sync::Arc;

    use selene_gql::ProcedureError;
    use selene_persist::WalReader;

    use crate::history::PackHistorySource;

    use super::{
        BuiltInMetadata, UNSTABLE_BUILTIN_CONTENT_HASH, create_index::SeleneCreateIndex,
        drop_index::SeleneDropIndex, health::SeleneHealth, pack_history::SelenePackHistory,
    };

    struct DummyHistory;

    impl PackHistorySource for DummyHistory {
        fn open_wal_reader(&self) -> Result<WalReader, ProcedureError> {
            panic!("metadata test must not execute pack history")
        }
    }

    #[test]
    fn builtin_content_hash_defaults_to_unstable_sentinel() {
        assert_eq!(SeleneHealth.content_hash(), UNSTABLE_BUILTIN_CONTENT_HASH);
    }

    #[test]
    fn every_platform_builtin_uses_unstable_content_hash_sentinel() {
        let history = SelenePackHistory::new(Arc::new(DummyHistory));
        let builtins: [&dyn BuiltInMetadata; 4] = [
            &SeleneHealth,
            &SeleneCreateIndex,
            &SeleneDropIndex,
            &history,
        ];

        for builtin in builtins {
            assert_eq!(builtin.content_hash(), UNSTABLE_BUILTIN_CONTENT_HASH);
        }
    }
}
