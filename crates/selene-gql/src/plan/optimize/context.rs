//! Optimizer context and statistics placeholders.

use std::{collections::HashMap, sync::OnceLock};

use selene_core::IStr;

use crate::plan::{ExecutionPlan, ImplDefinedCaps};

/// Context shared by all optimizer rules.
pub struct OptimizeContext<'a> {
    /// Implementation-defined planner limits.
    pub impl_defined_caps: &'a ImplDefinedCaps,
    /// Optional graph statistics populated by later optimizer work.
    pub statistics: Option<&'a EdgeStatistics>,
    /// Optional cardinality sampler populated by later execution work.
    pub sampler: Option<&'a dyn WanderJoinSampler>,
}

impl<'a> OptimizeContext<'a> {
    /// Construct a context using explicit implementation-defined limits.
    #[must_use]
    pub const fn new(impl_defined_caps: &'a ImplDefinedCaps) -> Self {
        Self {
            impl_defined_caps,
            statistics: None,
            sampler: None,
        }
    }

    /// Attach graph statistics to this context.
    #[must_use]
    pub const fn with_statistics(mut self, statistics: &'a EdgeStatistics) -> Self {
        self.statistics = Some(statistics);
        self
    }

    /// Attach a wander-join sampler to this context.
    #[must_use]
    pub const fn with_sampler(mut self, sampler: &'a dyn WanderJoinSampler) -> Self {
        self.sampler = Some(sampler);
        self
    }
}

static DEFAULT_CAPS: OnceLock<ImplDefinedCaps> = OnceLock::new();

fn default_caps() -> &'static ImplDefinedCaps {
    DEFAULT_CAPS.get_or_init(ImplDefinedCaps::default)
}

impl Default for OptimizeContext<'static> {
    fn default() -> Self {
        Self::new(default_caps())
    }
}

/// Skeleton statistics surface reserved for cost-aware optimizer rules.
#[derive(Clone, Debug, Default)]
pub struct EdgeStatistics {
    /// Number of nodes containing each label.
    pub label_node_counts: HashMap<IStr, u64>,
    /// Number of edges carrying each label.
    pub edge_label_counts: HashMap<IStr, u64>,
    /// Mean incoming degree per edge label.
    pub edge_in_degree: HashMap<IStr, f64>,
    /// Mean outgoing degree per edge label.
    pub edge_out_degree: HashMap<IStr, f64>,
    /// Per-label/property histograms reserved for later selectivity rules.
    pub property_histograms: HashMap<(IStr, IStr), PropertyHistogram>,
}

/// Placeholder histogram shape for future selectivity estimates.
#[derive(Clone, Debug, Default)]
pub struct PropertyHistogram {
    /// Histogram buckets. Bucket semantics are defined by later briefs.
    pub buckets: Vec<u64>,
}

/// Cardinality sampler placeholder used by future WCO and join-order rules.
pub trait WanderJoinSampler: Send + Sync {
    /// Estimate cardinality for a plan with a caller-provided sample budget.
    fn estimate(&self, plan: &ExecutionPlan, budget: u32) -> u64;
}
