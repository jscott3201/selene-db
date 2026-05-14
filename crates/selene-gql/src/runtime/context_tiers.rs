//! Procedure execution context tiers.

use std::sync::Arc;

use selene_graph::{IndexProvider, Mutator, ProviderTag, SeleneGraph};

use crate::{ImplDefinedCaps, ProcedureTier};

/// Read-tier procedure context.
pub struct GraphContext<'a> {
    snapshot: &'a SeleneGraph,
    caps: &'a ImplDefinedCaps,
    providers: &'a [Arc<dyn IndexProvider>],
}

impl<'a> GraphContext<'a> {
    pub(crate) const fn new(
        snapshot: &'a SeleneGraph,
        caps: &'a ImplDefinedCaps,
        providers: &'a [Arc<dyn IndexProvider>],
    ) -> Self {
        Self {
            snapshot,
            caps,
            providers,
        }
    }

    /// Borrow the graph snapshot visible to this procedure call.
    #[must_use]
    pub const fn snapshot(&self) -> &'a SeleneGraph {
        self.snapshot
    }

    /// Borrow implementation-defined executor caps.
    #[must_use]
    pub const fn impl_defined_caps(&self) -> &'a ImplDefinedCaps {
        self.caps
    }

    /// Look up a registered index provider by its fixed provider tag.
    #[must_use]
    pub fn index_provider_by_tag(&self, tag: ProviderTag) -> Option<Arc<dyn IndexProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.provider_tag() == tag)
            .map(Arc::clone)
    }
}

/// Mutation-tier procedure context.
pub struct MutationContext<'a, 'g> {
    mutator: Mutator<'a, 'g>,
    caps: &'a ImplDefinedCaps,
}

impl<'a, 'g> MutationContext<'a, 'g> {
    pub(crate) const fn new(mutator: Mutator<'a, 'g>, caps: &'a ImplDefinedCaps) -> Self {
        Self { mutator, caps }
    }

    /// Construct a mutation context for external procedure test harnesses.
    #[cfg(any(test, feature = "test-harness"))]
    #[must_use]
    pub fn for_test(mutator: Mutator<'a, 'g>, caps: &'a ImplDefinedCaps) -> Self {
        Self::new(mutator, caps)
    }

    /// Borrow the transaction-local working graph.
    #[must_use]
    pub fn snapshot(&self) -> &SeleneGraph {
        self.mutator.read()
    }

    /// Borrow the mutation funnel.
    pub fn mutator(&mut self) -> &mut Mutator<'a, 'g> {
        &mut self.mutator
    }

    /// Look up a registered index provider through the held write transaction.
    #[must_use]
    pub fn index_provider_by_tag(&self, tag: ProviderTag) -> Option<Arc<dyn IndexProvider>> {
        self.mutator.index_provider_by_tag(tag)
    }

    /// Borrow implementation-defined executor caps.
    #[must_use]
    pub const fn impl_defined_caps(&self) -> &'a ImplDefinedCaps {
        self.caps
    }
}

/// Tier-tagged procedure context passed through [`crate::ProcedureRegistry`].
#[non_exhaustive]
pub enum ProcedureContext<'a, 'g> {
    /// Read-only graph-tier context.
    Graph(GraphContext<'a>),
    /// Mutation-tier context.
    Mutation(MutationContext<'a, 'g>),
}

impl ProcedureContext<'_, '_> {
    /// Return the context tier represented by this enum variant.
    #[must_use]
    pub const fn tier(&self) -> ProcedureTier {
        match self {
            Self::Graph(_) => ProcedureTier::Graph,
            Self::Mutation(_) => ProcedureTier::Mutation,
        }
    }
}
