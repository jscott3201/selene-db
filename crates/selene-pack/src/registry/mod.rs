//! Frozen procedure-pack registry implementation.

mod storage;

use std::sync::Arc;

use selene_gql::{
    ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata, ProcedureRegistry,
    ProcedureResult, Value,
};

use crate::{
    builtin::{
        GraphProcedureBuiltIn, MutationProcedureBuiltIn, create_index::SeleneCreateIndex,
        drop_index::SeleneDropIndex, health::SeleneHealth, pack_history::SelenePackHistory,
    },
    error::RegistryError,
    history::PackHistorySource,
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

    /// Construct the standard platform built-in registry.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if a platform built-in fails to register —
    /// notably [`RegistryError::InternerCapExhausted`] when the embedder has
    /// already filled the global string interner. Embedders that want
    /// infallible-or-panic semantics can call `.expect(...)` at the call site;
    /// this surface intentionally does not panic so an operational resource
    /// limit is recoverable.
    pub fn with_builtins() -> Result<Self, RegistryError> {
        ProcedurePackRegistryBuilder::new()
            .with_graph_builtin(SeleneHealth)
            .with_mutation_builtin(SeleneCreateIndex)
            .with_mutation_builtin(SeleneDropIndex)
            .build()
    }

    /// Construct the standard platform built-in registry plus
    /// `selene.pack.history` reading from `history_source`.
    ///
    /// Use this constructor when the embedder has wired a WAL-writing provider
    /// and wants pack history queryable from GQL. Embedders that do not need
    /// history call [`Self::with_builtins`] instead; `selene.pack.history` is not
    /// registered there.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if a platform built-in fails to register.
    pub fn with_builtins_and_history(
        history_source: Arc<dyn PackHistorySource>,
    ) -> Result<Self, RegistryError> {
        ProcedurePackRegistryBuilder::new()
            .with_graph_builtin(SeleneHealth)
            .with_graph_builtin(SelenePackHistory::new(history_source))
            .with_mutation_builtin(SeleneCreateIndex)
            .with_mutation_builtin(SeleneDropIndex)
            .build()
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
///
/// `pub(crate)` for BRIEF-41 (Q8): v1.0 platform built-ins are the only
/// supported registration path. Manifest-driven extension registration
/// lands in BRIEF-43; the public registration API will be revisited then.
#[derive(Default)]
pub(crate) struct ProcedurePackRegistryBuilder {
    pending: Vec<PendingEntry>,
    next_handle: u64,
}

impl ProcedurePackRegistryBuilder {
    pub(crate) const fn new() -> Self {
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

    pub(crate) fn build(self) -> Result<ProcedurePackRegistry, RegistryError> {
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
        let err = ProcedurePackRegistryBuilder::new()
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
        let registry = ProcedurePackRegistryBuilder::new()
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
        let err = ProcedurePackRegistryBuilder::new()
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
        let err = ProcedurePackRegistryBuilder::new()
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
        let registry = ProcedurePackRegistryBuilder::new()
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

    /// Regression: BRIEF-41 round-1 Codex F1.
    /// Until BRIEF-48 lands real content hashes, two registrations of the
    /// same name MUST conflict even when both hashes are the `[0u8; 32]`
    /// placeholder, so a silently-dropped second implementation is
    /// impossible.
    #[test]
    fn duplicate_name_with_default_placeholder_hash_always_conflicts() {
        let err = ProcedurePackRegistryBuilder::new()
            .with_graph_builtin(TestGraphBuiltin {
                name: &["dup", "placeholder"],
                tier: ProcedureTier::Graph,
                hash: [0_u8; 32],
            })
            .with_graph_builtin(TestGraphBuiltin {
                name: &["dup", "placeholder"],
                tier: ProcedureTier::Graph,
                hash: [0_u8; 32],
            })
            .build()
            .expect_err("placeholder hash is never idempotent");

        assert!(matches!(err, RegistryError::Conflict { .. }));
    }

    /// Regression: BRIEF-41 round-1 Codex F3.
    /// A built-in declaring `Graph` tier with a write mutability surfaces
    /// at build time rather than slipping through to a runtime
    /// `ProcedureError::TierMismatch` from BRIEF-39's executor-side
    /// `validate_call_tier`.
    #[test]
    fn mutability_inconsistent_with_declared_tier_returns_mutability_tier_mismatch() {
        #[derive(Clone, Copy)]
        struct GraphTierGraphWriteMutability;

        impl BuiltInMetadata for GraphTierGraphWriteMutability {
            fn name(&self) -> &'static [&'static str] {
                &["inconsistent", "mutability"]
            }
            fn tier(&self) -> ProcedureTier {
                ProcedureTier::Graph
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

        impl GraphProcedureBuiltIn for GraphTierGraphWriteMutability {
            fn execute(
                &self,
                _ctx: &selene_gql::GraphContext<'_>,
                _args: &[Value],
            ) -> Result<ProcedureResult, ProcedureError> {
                Ok(ProcedureResult { rows: Vec::new() })
            }
        }

        let err = ProcedurePackRegistryBuilder::new()
            .with_graph_builtin(GraphTierGraphWriteMutability)
            .build()
            .expect_err("graph tier + GraphWrite mutability is inconsistent");

        assert!(matches!(
            err,
            RegistryError::MutabilityTierMismatch {
                declared_tier: ProcedureTier::Graph,
                declared_mutability: ProcedureMutability::GraphWrite,
                expected_tier: ProcedureTier::Mutation,
                ..
            }
        ));
    }

    /// Regression: BRIEF-41 round-1 Codex F5.
    /// `with_builtins()` returns `Result` so an `InternerCapExhausted`
    /// surface is recoverable. The happy path returns the platform
    /// registry; embedders that want infallible-or-panic semantics call
    /// `.expect(...)` at their call site, not from inside selene-pack.
    #[test]
    fn with_builtins_returns_ok_on_clean_interner() {
        let registry = ProcedurePackRegistry::with_builtins()
            .expect("platform built-ins register cleanly in tests");
        let name = [
            selene_core::intern("selene").expect("interns"),
            selene_core::intern("health").expect("interns"),
        ];
        assert!(registry.lookup(&name).is_some());
    }
}
