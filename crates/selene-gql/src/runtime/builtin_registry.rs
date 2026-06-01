//! Concrete native built-in procedure registry.
//!
//! [`BuiltinProcedureRegistry`] is the single production implementation of the
//! [`ProcedureRegistry`] trait (D16, decision (c)). It binds `CALL algo.*`
//! directly over the `selene-algorithms` native API — no `ExternalGraphProcedure`
//! indirection, no loadable-pack machinery. The registry is **frozen at
//! construction**: it allocates a fixed set of opaque handles, builds the
//! name→metadata and handle→dispatch tables once, and exposes
//! [`registry_version`](ProcedureRegistry::registry_version) as a constant `0`
//! so the shared CALL plan cache ([`crate::CallPlanCache`]) key stays stable
//! across statements.
//!
//! STEP 2 registers the 19 `algo.*` procedures. The 5 surviving platform
//! built-ins (`selene.health`, `selene.feature_status`, `selene.verify`,
//! `selene.create_index`, `selene.drop_index`) are relocated here in STEP 3,
//! bringing the total to 24; the registry's tables and `iter_handles` are
//! already shaped to carry both.

use std::collections::HashMap;

use selene_core::{GraphId, IStr, Value, intern};

use crate::ProcedureContext;
use crate::procedure_registry::{
    ProcedureError, ProcedureHandle, ProcedureMetadata, ProcedureRegistry, ProcedureResult,
};
use crate::runtime::builtins::{BUILTIN_SPECS, BuiltinKind};
use crate::runtime::native_algorithms::{ALGO_SPECS, AlgoKind, AlgorithmCatalogs, forget_graph};

/// Frozen registry version. Construction-once registries keep `0` so the shared
/// CALL plan cache never invalidates against a version bump (risk #5).
const REGISTRY_VERSION: u64 = 0;

/// What an opaque [`ProcedureHandle`] dispatches to.
#[derive(Clone, Copy, Debug)]
enum Dispatch {
    /// A native `algo.*` procedure.
    Algo(AlgoKind),
    /// A native `selene.*` platform built-in.
    Builtin(BuiltinKind),
}

/// Concrete native procedure registry — the single production
/// [`ProcedureRegistry`] impl.
#[derive(Debug)]
pub struct BuiltinProcedureRegistry {
    /// `name → metadata`, used by plan-time [`lookup`](ProcedureRegistry::lookup).
    by_name: HashMap<Box<[IStr]>, ProcedureMetadata>,
    /// `handle → dispatch`, used by runtime [`execute`](ProcedureRegistry::execute).
    by_handle: HashMap<ProcedureHandle, Dispatch>,
    /// `(name, metadata)` pairs in registration order for
    /// [`iter_handles`](ProcedureRegistry::iter_handles) (SHOW PROCEDURES).
    ordered: Vec<(Vec<IStr>, ProcedureMetadata)>,
    /// Engine-internal, per-`GraphId`, ephemeral projection catalogs.
    catalogs: AlgorithmCatalogs,
}

impl BuiltinProcedureRegistry {
    /// Construct the frozen native registry with the platform procedure set.
    ///
    /// # Panics
    ///
    /// Panics only if the global string interner is exhausted while interning
    /// the fixed, static procedure-name segments. The set is small and known at
    /// compile time; an interner with any spare capacity at engine startup
    /// admits it. (The historical pack surfaced this as a recoverable
    /// `InternerCapExhausted`; the native registry's name set is a closed
    /// compile-time constant, so exhaustion here is a startup-environment bug,
    /// not an operational input.)
    #[must_use]
    pub fn new() -> Self {
        let mut by_name = HashMap::new();
        let mut by_handle = HashMap::new();
        let mut ordered = Vec::new();

        // Handles are 1-based and assigned in registration order: the 19
        // `algo.*` procedures first (handles 1..=19), then the 5 `selene.*`
        // platform built-ins (handles 20..=24), continuing the same monotonic
        // sequence. `next_handle` carries the running 1-based handle value.
        let mut next_handle = 1_u64;
        for spec in &ALGO_SPECS {
            let handle = ProcedureHandle::new(next_handle);
            next_handle += 1;
            let name = intern_name(spec.name);
            let metadata = spec.kind.metadata(handle, spec.description);

            by_handle.insert(handle, Dispatch::Algo(spec.kind));
            by_name.insert(name.clone().into_boxed_slice(), metadata.clone());
            ordered.push((name, metadata));
        }
        for spec in &BUILTIN_SPECS {
            let handle = ProcedureHandle::new(next_handle);
            next_handle += 1;
            let name = intern_name(spec.name);
            let metadata = spec
                .kind
                .metadata(handle, spec.description, spec.since_version);

            by_handle.insert(handle, Dispatch::Builtin(spec.kind));
            by_name.insert(name.clone().into_boxed_slice(), metadata.clone());
            ordered.push((name, metadata));
        }

        Self {
            by_name,
            by_handle,
            ordered,
            catalogs: AlgorithmCatalogs::default(),
        }
    }

    /// Reclaim the ephemeral projection catalog for a dropped graph.
    ///
    /// Projections are derived, never-persisted state scoped per `GraphId`; an
    /// embedder calls this when a graph is dropped so a later `GraphId` reuse
    /// cannot observe stale projections. Returns `true` if state was present.
    pub fn forget_graph(&self, graph_id: GraphId) -> bool {
        forget_graph(&self.catalogs, graph_id)
    }
}

impl Default for BuiltinProcedureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcedureRegistry for BuiltinProcedureRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        self.by_name.get(name).cloned()
    }

    fn registry_version(&self) -> u64 {
        REGISTRY_VERSION
    }

    fn iter_handles(&self) -> Box<dyn Iterator<Item = (Vec<IStr>, ProcedureMetadata)> + '_> {
        Box::new(
            self.ordered
                .iter()
                .map(|(name, metadata)| (name.clone(), metadata.clone())),
        )
    }

    fn execute(
        &self,
        handle: ProcedureHandle,
        args: &[Value],
        ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        let _span = tracing::span!(
            tracing::Level::INFO,
            "selene.procedure.dispatch",
            procedure = tracing::field::Empty
        )
        .entered();

        let Some(dispatch) = self.by_handle.get(&handle).copied() else {
            return Err(ProcedureError::UnknownProcedure { name: Box::new([]) });
        };

        match dispatch {
            Dispatch::Algo(kind) => {
                tracing::Span::current()
                    .record("procedure", tracing::field::display(procedure_name(kind)));
                // Algorithm procedures are read-only graph-tier; a mutation-tier
                // context is a tier mismatch (mirrors the pack registry's
                // tier-checked dispatch).
                let ProcedureContext::Graph(graph_ctx) = ctx else {
                    return Err(ProcedureError::TierMismatch {
                        expected: crate::ProcedureTier::Graph,
                        actual: ctx.tier(),
                    });
                };
                kind.execute(&self.catalogs, graph_ctx, args)
            }
            Dispatch::Builtin(kind) => {
                tracing::Span::current()
                    .record("procedure", tracing::field::display(builtin_name(kind)));
                // Each built-in declares its own tier (read-only graph-tier for
                // health/feature_status/verify; mutation-tier for
                // create_index/drop_index). Route through the matching context;
                // a mismatch is a tier error (the planner's plan-time tier check
                // already rejects the common cases, but the registry stays
                // self-consistent — and the mutation-tier built-ins write only
                // through `MutationContext::mutator`, never bypassing the funnel).
                match (kind.tier(), ctx) {
                    (crate::ProcedureTier::Graph, ProcedureContext::Graph(graph_ctx)) => {
                        kind.execute_graph(graph_ctx, args)
                    }
                    (crate::ProcedureTier::Mutation, ProcedureContext::Mutation(mut_ctx)) => {
                        kind.execute_mutation(mut_ctx, args)
                    }
                    (expected, ctx) => Err(ProcedureError::TierMismatch {
                        expected,
                        actual: ctx.tier(),
                    }),
                }
            }
        }
    }
}

/// Best-effort dotted name for tracing, derived from the static spec table.
fn procedure_name(kind: AlgoKind) -> String {
    ALGO_SPECS
        .iter()
        .find(|spec| spec.kind == kind)
        .map_or_else(String::new, |spec| spec.name.join("."))
}

/// Best-effort dotted name for tracing a platform built-in.
fn builtin_name(kind: BuiltinKind) -> String {
    BUILTIN_SPECS
        .iter()
        .find(|spec| spec.kind == kind)
        .map_or_else(String::new, |spec| spec.name.join("."))
}

fn intern_name(raw: &'static [&'static str]) -> Vec<IStr> {
    raw.iter()
        .map(|segment| intern(segment).expect("static procedure name segment interns"))
        .collect()
}

#[cfg(test)]
mod tests {
    use selene_core::intern;

    use super::*;
    use crate::{ProcedureMutability, ProcedureTier};

    fn name(segments: &[&str]) -> Vec<IStr> {
        segments
            .iter()
            .map(|segment| intern(segment).expect("interns"))
            .collect()
    }

    #[test]
    fn registers_all_twenty_four_procedures() {
        let registry = BuiltinProcedureRegistry::new();
        let handles: Vec<_> = registry.iter_handles().collect();
        assert_eq!(
            handles.len(),
            24,
            "expected 19 algo procedures + 5 platform built-ins"
        );
    }

    #[test]
    fn iter_handles_yields_all_five_platform_builtins() {
        let registry = BuiltinProcedureRegistry::new();
        let names: Vec<Vec<String>> = registry
            .iter_handles()
            .map(|(name, _)| {
                name.iter()
                    .map(|segment| segment.as_str().to_owned())
                    .collect()
            })
            .collect();
        for expected in [
            ["selene", "health"],
            ["selene", "feature_status"],
            ["selene", "verify"],
            ["selene", "create_index"],
            ["selene", "drop_index"],
        ] {
            let expected: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
            assert!(
                names.contains(&expected),
                "SHOW PROCEDURES must list {expected:?}"
            );
        }
    }

    #[test]
    fn builtin_tiers_and_mutability_match_pack() {
        let registry = BuiltinProcedureRegistry::new();
        // Read-only graph-tier built-ins.
        for builtin in [
            &["selene", "health"][..],
            &["selene", "feature_status"][..],
            &["selene", "verify"][..],
        ] {
            let metadata = registry.lookup(&name(builtin)).expect("resolves");
            assert_eq!(metadata.tier, ProcedureTier::Graph, "{builtin:?}");
            assert_eq!(
                metadata.mutability,
                ProcedureMutability::Read,
                "{builtin:?}"
            );
        }
        // Mutation-tier schema-write built-ins.
        for builtin in [
            &["selene", "create_index"][..],
            &["selene", "drop_index"][..],
        ] {
            let metadata = registry.lookup(&name(builtin)).expect("resolves");
            assert_eq!(metadata.tier, ProcedureTier::Mutation, "{builtin:?}");
            assert_eq!(
                metadata.mutability,
                ProcedureMutability::SchemaWrite,
                "{builtin:?}"
            );
        }
    }

    #[test]
    fn verify_signature_has_optional_deep_arg() {
        let registry = BuiltinProcedureRegistry::new();
        let metadata = registry
            .lookup(&name(&["selene", "verify"]))
            .expect("verify resolves");
        // Single `deep` BOOLEAN parameter carrying an executable default, so the
        // arity accepts 0 or 1 args (matching the pack's `with_default`).
        assert_eq!(metadata.signature.parameters.len(), 1);
        let arity = metadata.signature.arity();
        assert_eq!(arity.minimum, 0);
        assert_eq!(arity.maximum, 1);
        let deep = &metadata.signature.parameters[0];
        assert_eq!(deep.default_doc, Some("false"));
        assert!(deep.default.is_some());
    }

    #[test]
    fn create_index_signature_is_exact_three_args() {
        let registry = BuiltinProcedureRegistry::new();
        let metadata = registry
            .lookup(&name(&["selene", "create_index"]))
            .expect("create_index resolves");
        let arity = metadata.signature.arity();
        assert_eq!(arity.minimum, 3);
        assert_eq!(arity.maximum, 3);
        assert!(metadata.output_schema.columns.is_empty());
    }

    #[test]
    fn registry_version_is_frozen_zero() {
        assert_eq!(BuiltinProcedureRegistry::new().registry_version(), 0);
    }

    #[test]
    fn every_algo_procedure_resolves_by_name() {
        let registry = BuiltinProcedureRegistry::new();
        for spec in &ALGO_SPECS {
            let key = name(spec.name);
            assert!(
                registry.lookup(&key).is_some(),
                "procedure {:?} must resolve",
                spec.name
            );
        }
    }

    #[test]
    fn algo_procedures_are_graph_tier_read() {
        let registry = BuiltinProcedureRegistry::new();
        for spec in &ALGO_SPECS {
            let metadata = registry.lookup(&name(spec.name)).expect("resolves");
            assert_eq!(metadata.tier, ProcedureTier::Graph, "{:?}", spec.name);
            assert_eq!(
                metadata.mutability,
                ProcedureMutability::Read,
                "{:?}",
                spec.name
            );
        }
    }

    #[test]
    fn handles_are_unique_and_one_based() {
        let registry = BuiltinProcedureRegistry::new();
        let mut handles: Vec<u64> = registry
            .iter_handles()
            .map(|(_, metadata)| metadata.handle.raw())
            .collect();
        handles.sort_unstable();
        assert_eq!(handles, (1..=24).collect::<Vec<_>>());
    }

    #[test]
    fn unknown_name_returns_none() {
        let registry = BuiltinProcedureRegistry::new();
        assert!(registry.lookup(&name(&["algo", "nonexistent"])).is_none());
    }

    #[test]
    fn pagerank_signature_matches_pack_shape() {
        let registry = BuiltinProcedureRegistry::new();
        let metadata = registry
            .lookup(&name(&["algo", "pagerank"]))
            .expect("pagerank resolves");
        // projection_name, damping, max_iterations, tolerance, parallelism
        assert_eq!(metadata.signature.parameters.len(), 5);
        // The pack attached `default_doc` (not an executable `default`) to the
        // nullable params, so arity is exact (5..5): the CALL site must still
        // pass all five positional args; nullability is enforced per-arg, not by
        // omission. Preserving this keeps the procedure signature unchanged.
        let arity = metadata.signature.arity();
        assert_eq!(arity.minimum, 5);
        assert_eq!(arity.maximum, 5);
        // The four optional params are nullable and carry the pack default-doc.
        for parameter in &metadata.signature.parameters[1..] {
            assert!(parameter.nullable, "{} should be nullable", parameter.name);
            assert_eq!(parameter.default_doc, Some("NULL (use procedure default)"));
            assert!(parameter.default.is_none());
        }
        // Output columns: node_id, score.
        assert_eq!(metadata.output_schema.columns.len(), 2);
        assert_eq!(metadata.output_schema.columns[0].name.as_str(), "node_id");
        assert_eq!(metadata.output_schema.columns[1].name.as_str(), "score");
    }

    #[test]
    fn forget_graph_is_idempotent() {
        let registry = BuiltinProcedureRegistry::new();
        // No catalog has been created yet for this id.
        assert!(!registry.forget_graph(GraphId::new(7)));
    }
}
