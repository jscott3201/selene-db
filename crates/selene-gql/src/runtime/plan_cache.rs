//! Session-scoped execution-plan cache.

use std::{borrow::Borrow, num::NonZeroUsize, sync::Arc};

use lru::LruCache;

use crate::{ExecutionPlan, PipelineOp};

/// LRU-cached execution plans keyed by source text.
pub struct PlanCache {
    inner: LruCache<CacheKey, CachedPlan>,
    stats: PlanCacheStats,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    source: Arc<str>,
}

impl Borrow<str> for CacheKey {
    fn borrow(&self) -> &str {
        &self.source
    }
}

struct CachedPlan {
    plan: Arc<ExecutionPlan>,
    schema_version_at_plan: u64,
}

/// Counters for [`PlanCache`] lookup and eviction behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlanCacheStats {
    /// Successful source + schema-version lookups.
    pub hits: u64,
    /// Source keys not present in the cache.
    pub misses: u64,
    /// Entries found but invalidated because their schema version was stale.
    pub stale_invalidations: u64,
    /// Entries evicted by LRU capacity pressure.
    pub capacity_evictions: u64,
}

impl PlanCache {
    /// Create an empty plan cache with the given entry capacity.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: LruCache::new(capacity),
            stats: PlanCacheStats::default(),
        }
    }

    pub(crate) fn get(&mut self, source: &str, schema_version: u64) -> Option<Arc<ExecutionPlan>> {
        match self.inner.get(source) {
            Some(cached) if cached.schema_version_at_plan == schema_version => {
                let plan = Arc::clone(&cached.plan);
                self.stats.hits = self.stats.hits.saturating_add(1);
                trace_cache_event("hit", schema_version, source);
                Some(plan)
            }
            Some(_) => {
                self.inner.pop(source);
                self.stats.stale_invalidations = self.stats.stale_invalidations.saturating_add(1);
                trace_cache_event("stale", schema_version, source);
                None
            }
            None => {
                self.stats.misses = self.stats.misses.saturating_add(1);
                trace_cache_event("miss", schema_version, source);
                None
            }
        }
    }

    pub(crate) fn insert(
        &mut self,
        source: Arc<str>,
        plan: Arc<ExecutionPlan>,
        schema_version: u64,
    ) {
        if !is_cacheable(&plan) {
            return;
        }

        let replacing_existing = self.inner.contains(source.as_ref());
        let key = CacheKey { source };
        let cached = CachedPlan {
            plan,
            schema_version_at_plan: schema_version,
        };
        if self.inner.push(key, cached).is_some() && !replacing_existing {
            self.stats.capacity_evictions = self.stats.capacity_evictions.saturating_add(1);
        }
    }

    /// Return a snapshot of the cache counters.
    #[must_use]
    pub const fn stats(&self) -> PlanCacheStats {
        self.stats
    }

    /// Remove all cached plans while preserving accumulated counters.
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

fn is_cacheable(plan: &ExecutionPlan) -> bool {
    !contains_call(plan)
}

fn contains_call(plan: &ExecutionPlan) -> bool {
    plan.pipeline.iter().any(op_contains_call)
}

fn op_contains_call(op: &PipelineOp) -> bool {
    match op {
        PipelineOp::Call(_) => true,
        PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => contains_call(rhs),
        _ => false,
    }
}

fn trace_cache_event(result: &'static str, schema_version: u64, source: &str) {
    #[cfg(feature = "plan-cache-source-tracing")]
    tracing::debug!(
        result,
        schema_version,
        source_len = source.len(),
        source_prefix = %SourcePrefix(source),
        "session plan cache lookup"
    );

    #[cfg(not(feature = "plan-cache-source-tracing"))]
    tracing::debug!(
        result,
        schema_version,
        source_len = source.len(),
        "session plan cache lookup"
    );
}

#[cfg(feature = "plan-cache-source-tracing")]
struct SourcePrefix<'a>(&'a str);

#[cfg(feature = "plan-cache-source-tracing")]
impl std::fmt::Display for SourcePrefix<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut chars = self.0.chars();
        let prefix = chars.by_ref().take(200).collect::<String>();
        if chars.next().is_some() {
            write!(formatter, "{prefix}...")
        } else {
            formatter.write_str(&prefix)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc};

    use selene_core::{IStr, intern_with_admission};

    use super::*;
    use crate::{
        BindingTableSchema, EmptyProcedureRegistry, ExprId, ImplDefinedCaps, PipelineOpId,
        PlannedCall, ProcedureHandle, ProcedureMutability, ProcedureOutputSchema, ProcedureTier,
        SourceSpan, StatementCategory, analyze::analyze, parser::parse, plan::plan,
    };

    fn planned(source: &str) -> Arc<ExecutionPlan> {
        let statement = parse(source).expect("test source parses");
        let analyzed =
            analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes");
        Arc::new(plan(&analyzed, &EmptyProcedureRegistry).expect("test source plans"))
    }

    fn admitted(value: &str) -> IStr {
        intern_with_admission(value).expect("test name admits").0
    }

    fn call_plan() -> Arc<ExecutionPlan> {
        Arc::new(ExecutionPlan {
            category: StatementCategory::ReadOnly,
            pattern_plan: None,
            pipeline: vec![PipelineOp::Call(PlannedCall {
                procedure: Box::from([admitted("cache"), admitted("call")]),
                handle: ProcedureHandle::new(1),
                args: Vec::new(),
                yield_cols: Vec::new(),
                output_schema: ProcedureOutputSchema::default(),
                yield_schema: Vec::new(),
                tier: ProcedureTier::Graph,
                mutability: ProcedureMutability::Read,
                span: SourceSpan::default(),
            })],
            output_schema: BindingTableSchema {
                columns: Vec::new(),
            },
            impl_defined_caps: ImplDefinedCaps::default(),
            expr_ids: Default::default(),
            subqueries: Default::default(),
            next_expr_id: ExprId::new(0),
            next_pipeline_op_id: PipelineOpId::new(1),
        })
    }

    #[test]
    fn plan_cache_basic_hit_miss() {
        let mut cache = PlanCache::new(NonZeroUsize::new(2).unwrap());
        assert!(cache.get("RETURN 1", 0).is_none());

        cache.insert(Arc::from("RETURN 1"), planned("RETURN 1"), 0);
        assert!(cache.get("RETURN 1", 0).is_some());

        assert_eq!(
            cache.stats(),
            PlanCacheStats {
                hits: 1,
                misses: 1,
                stale_invalidations: 0,
                capacity_evictions: 0,
            }
        );
    }

    #[test]
    fn plan_cache_lru_evicts_oldest() {
        let mut cache = PlanCache::new(NonZeroUsize::new(2).unwrap());
        cache.insert(Arc::from("RETURN 1"), planned("RETURN 1"), 0);
        cache.insert(Arc::from("RETURN 2"), planned("RETURN 2"), 0);
        cache.insert(Arc::from("RETURN 3"), planned("RETURN 3"), 0);

        assert!(cache.get("RETURN 1", 0).is_none());
        assert!(cache.get("RETURN 2", 0).is_some());
        assert!(cache.get("RETURN 3", 0).is_some());
        assert_eq!(cache.stats().capacity_evictions, 1);
    }

    #[test]
    fn plan_cache_schema_version_mismatch_is_miss() {
        let mut cache = PlanCache::new(NonZeroUsize::new(2).unwrap());
        cache.insert(Arc::from("RETURN 1"), planned("RETURN 1"), 0);

        assert!(cache.get("RETURN 1", 1).is_none());
        assert_eq!(cache.stats().stale_invalidations, 1);
        assert!(cache.get("RETURN 1", 1).is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn plan_cache_clear_resets_state() {
        let mut cache = PlanCache::new(NonZeroUsize::new(2).unwrap());
        cache.insert(Arc::from("RETURN 1"), planned("RETURN 1"), 0);
        assert!(cache.get("RETURN 1", 0).is_some());

        cache.clear();
        assert!(cache.get("RETURN 1", 0).is_none());

        assert_eq!(
            cache.stats(),
            PlanCacheStats {
                hits: 1,
                misses: 1,
                stale_invalidations: 0,
                capacity_evictions: 0,
            }
        );
    }

    #[test]
    fn cache_skips_call_plans() {
        let mut cache = PlanCache::new(NonZeroUsize::new(2).unwrap());
        cache.insert(Arc::from("CALL cache.call()"), call_plan(), 0);

        assert!(cache.get("CALL cache.call()", 0).is_none());
        assert_eq!(cache.stats().misses, 1);
    }
}
