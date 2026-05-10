//! Frozen procedure-pack registry implementation.

mod storage;

use std::sync::Arc;

use selene_gql::{
    ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata, ProcedureRegistry,
    ProcedureResult, Value,
};

use crate::{
    builtin::{GraphProcedureBuiltIn, MutationProcedureBuiltIn, health::SeleneHealth},
    error::RegistryError,
};

use storage::{PendingEntry, RegistryStorage, TierEntry};

/// Frozen procedure-pack registry.
///
/// The registry is read-only after construction. Internally it uses papaya maps
/// for lock-free plan-time lookup and runtime handle dispatch.
#[derive(Clone, Debug)]
pub struct ProcedurePackRegistry {
    storage: Arc<RegistryStorage>,
}

impl ProcedurePackRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            storage: Arc::new(RegistryStorage::empty()),
        }
    }

    /// Return a builder for a custom registry.
    #[must_use]
    pub const fn builder() -> ProcedurePackRegistryBuilder {
        ProcedurePackRegistryBuilder::new()
    }

    /// Construct the standard platform built-in registry.
    #[must_use]
    pub fn with_builtins() -> Self {
        Self::builder()
            .with_graph_builtin(SeleneHealth)
            .build()
            .expect("platform built-ins are valid")
    }
}

impl Default for ProcedurePackRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl ProcedureRegistry for ProcedurePackRegistry {
    fn lookup(&self, name: &[selene_core::IStr]) -> Option<ProcedureMetadata> {
        self.storage.lookup(name)
    }

    fn execute(
        &self,
        handle: ProcedureHandle,
        args: &[Value],
        ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        let Some(entry) = self.storage.entry(handle) else {
            return Err(ProcedureError::UnknownProcedure { name: Box::new([]) });
        };

        let actual_tier = ctx.tier();
        match (&entry, ctx) {
            (TierEntry::Graph(procedure), ProcedureContext::Graph(graph_ctx)) => {
                procedure.execute(graph_ctx, args)
            }
            (TierEntry::Mutation(procedure), ProcedureContext::Mutation(mutation_ctx)) => {
                procedure.execute(mutation_ctx, args)
            }
            _ => Err(ProcedureError::TierMismatch {
                expected: entry.tier(),
                actual: actual_tier,
            }),
        }
    }
}

/// Construct-once procedure-pack registry builder.
///
/// Built-ins are accepted only before [`build`](Self::build). The frozen
/// [`ProcedurePackRegistry`] retains no handle allocator or registration API.
#[derive(Default)]
pub struct ProcedurePackRegistryBuilder {
    pending: Vec<PendingEntry>,
    next_handle: u64,
}

impl ProcedurePackRegistryBuilder {
    /// Construct an empty builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: Vec::new(),
            next_handle: 1,
        }
    }

    pub(crate) fn with_graph_builtin<T>(mut self, builtin: T) -> Self
    where
        T: GraphProcedureBuiltIn,
    {
        self.pending.push(PendingEntry::graph(
            ProcedureHandle::new(self.next_handle),
            builtin,
        ));
        self.next_handle += 1;
        self
    }

    #[allow(dead_code)] // Reserved for BRIEF-42's first mutation-tier built-in.
    pub(crate) fn with_mutation_builtin<T>(mut self, builtin: T) -> Self
    where
        T: MutationProcedureBuiltIn,
    {
        self.pending.push(PendingEntry::mutation(
            ProcedureHandle::new(self.next_handle),
            builtin,
        ));
        self.next_handle += 1;
        self
    }

    /// Freeze this builder into a runtime registry.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] when static built-in metadata is malformed,
    /// conflicts with another registration, or attempts to register a tier that
    /// v1.0 does not support.
    pub fn build(self) -> Result<ProcedurePackRegistry, RegistryError> {
        Ok(ProcedurePackRegistry {
            storage: Arc::new(RegistryStorage::from_pending(self.pending)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use selene_gql::{
        GqlType, MutationContext, ProcedureError, ProcedureMutability, ProcedureResult,
        ProcedureTier, Value,
    };

    use crate::builtin::{
        BuiltInMetadata, GraphProcedureBuiltIn, MutationProcedureBuiltIn, StaticOutputColumn,
        StaticParameter,
    };

    use super::*;

    static OUTPUTS: [StaticOutputColumn; 1] = [StaticOutputColumn {
        name: "out",
        ty: GqlType::Integer,
    }];

    #[derive(Clone, Copy)]
    struct TestGraphBuiltin {
        name: &'static [&'static str],
        tier: ProcedureTier,
        hash: [u8; 32],
    }

    impl BuiltInMetadata for TestGraphBuiltin {
        fn name(&self) -> &'static [&'static str] {
            self.name
        }

        fn tier(&self) -> ProcedureTier {
            self.tier
        }

        fn mutability(&self) -> ProcedureMutability {
            ProcedureMutability::Read
        }

        fn signature_static(&self) -> &'static [StaticParameter] {
            &[]
        }

        fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
            &OUTPUTS
        }

        fn content_hash(&self) -> [u8; 32] {
            self.hash
        }
    }

    impl GraphProcedureBuiltIn for TestGraphBuiltin {
        fn execute(
            &self,
            _ctx: &selene_gql::GraphContext<'_>,
            _args: &[Value],
        ) -> Result<ProcedureResult, ProcedureError> {
            Ok(ProcedureResult { rows: Vec::new() })
        }
    }

    #[derive(Clone, Copy)]
    struct TestMutationBuiltin;

    impl BuiltInMetadata for TestMutationBuiltin {
        fn name(&self) -> &'static [&'static str] {
            &["test", "mutation"]
        }

        fn tier(&self) -> ProcedureTier {
            ProcedureTier::Mutation
        }

        fn mutability(&self) -> ProcedureMutability {
            ProcedureMutability::GraphWrite
        }

        fn signature_static(&self) -> &'static [StaticParameter] {
            &[]
        }

        fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
            &OUTPUTS
        }
    }

    impl MutationProcedureBuiltIn for TestMutationBuiltin {
        fn execute(
            &self,
            _ctx: &mut MutationContext<'_, '_>,
            _args: &[Value],
        ) -> Result<ProcedureResult, ProcedureError> {
            Ok(ProcedureResult { rows: Vec::new() })
        }
    }

    #[test]
    fn duplicate_name_with_different_hash_errors() {
        let err = ProcedurePackRegistry::builder()
            .with_graph_builtin(TestGraphBuiltin {
                name: &["dup", "proc"],
                tier: ProcedureTier::Graph,
                hash: [1_u8; 32],
            })
            .with_graph_builtin(TestGraphBuiltin {
                name: &["dup", "proc"],
                tier: ProcedureTier::Graph,
                hash: [2_u8; 32],
            })
            .build()
            .expect_err("duplicate name with different hash errors");

        let RegistryError::Conflict {
            existing_hash,
            new_hash,
            ..
        } = err
        else {
            panic!("expected conflict error");
        };
        assert_eq!(existing_hash, [1_u8; 32]);
        assert_eq!(new_hash, [2_u8; 32]);
    }

    #[test]
    fn duplicate_name_with_same_hash_is_idempotent() {
        let registry = ProcedurePackRegistry::builder()
            .with_graph_builtin(TestGraphBuiltin {
                name: &["dup", "same"],
                tier: ProcedureTier::Graph,
                hash: [3_u8; 32],
            })
            .with_graph_builtin(TestGraphBuiltin {
                name: &["dup", "same"],
                tier: ProcedureTier::Graph,
                hash: [3_u8; 32],
            })
            .build()
            .expect("same content hash is idempotent");

        let name = [
            selene_core::intern("dup").expect("interns"),
            selene_core::intern("same").expect("interns"),
        ];
        let metadata = registry.lookup(&name).expect("procedure registered once");
        assert_eq!(metadata.handle, ProcedureHandle::new(1));
    }

    #[test]
    fn register_persist_tier_returns_persist_tier_not_in_v1() {
        let err = ProcedurePackRegistry::builder()
            .with_graph_builtin(TestGraphBuiltin {
                name: &["persist", "later"],
                tier: ProcedureTier::Persist,
                hash: [0_u8; 32],
            })
            .build()
            .expect_err("persist tier rejected");

        assert!(matches!(err, RegistryError::PersistTierNotInV1 { .. }));
    }

    #[test]
    fn tier_mismatch_reports_declared_and_attempted_tiers() {
        let err = ProcedurePackRegistry::builder()
            .with_graph_builtin(TestGraphBuiltin {
                name: &["wrong", "tier"],
                tier: ProcedureTier::Mutation,
                hash: [0_u8; 32],
            })
            .build()
            .expect_err("declared tier does not match builder");

        assert!(matches!(
            err,
            RegistryError::TierMismatch {
                declared: ProcedureTier::Mutation,
                attempted: ProcedureTier::Graph,
                ..
            }
        ));
    }

    #[test]
    fn mutation_builtin_registration_uses_mutation_tier() {
        let registry = ProcedurePackRegistry::builder()
            .with_mutation_builtin(TestMutationBuiltin)
            .build()
            .expect("mutation built-in registers");
        let name = [
            selene_core::intern("test").expect("interns"),
            selene_core::intern("mutation").expect("interns"),
        ];

        let metadata = registry.lookup(&name).expect("mutation procedure exists");

        assert_eq!(metadata.tier, ProcedureTier::Mutation);
    }
}
