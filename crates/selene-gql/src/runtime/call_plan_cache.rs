//! Shared execution-plan cache for procedure-call-rooted statements.

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex, MutexGuard},
};

use lru::LruCache;
use selene_core::GraphId;

use crate::{
    ExecutionPlan, PipelineStatement,
    ast::{Statement, format_procedure_call, format_read_statement},
};

/// Shared LRU cache for procedure-call execution plans.
///
/// The cache is caller-owned so an embedder can share one
/// `Arc<CallPlanCache>` across all sessions executing against the same graph.
pub struct CallPlanCache {
    inner: Mutex<CallPlanCacheInner>,
}

struct CallPlanCacheInner {
    plans: LruCache<CallPlanKey, Arc<ExecutionPlan>>,
    source_index: LruCache<Arc<str>, Vec<CallPlanSourceEntry>>,
    stats: CallPlanCacheStats,
}

/// Stable key for a cached procedure-call plan.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CallPlanKey {
    graph_id: GraphId,
    schema_version: u64,
    registry_version: u64,
    canonical_source: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CallPlanSourceEntry {
    graph_id: GraphId,
    schema_version: u64,
    registry_version: u64,
    key: CallPlanKey,
}

/// Counters for [`CallPlanCache`] lookup and eviction behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CallPlanCacheStats {
    /// Successful key lookups.
    pub hits: u64,
    /// Keys not present in the cache.
    pub misses: u64,
    /// Entries evicted by LRU capacity pressure.
    pub capacity_evictions: u64,
}

impl CallPlanCache {
    /// Create an empty shared CALL plan cache with the given entry capacity.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: Mutex::new(CallPlanCacheInner {
                plans: LruCache::new(capacity),
                source_index: LruCache::new(capacity),
                stats: CallPlanCacheStats::default(),
            }),
        }
    }

    pub(crate) fn get_source(
        &self,
        graph_id: GraphId,
        schema_version: u64,
        registry_version: u64,
        source: &str,
    ) -> Option<Arc<ExecutionPlan>> {
        let mut inner = self.lock_inner();
        let Some(key) = inner.source_index.get(source).and_then(|entries| {
            entries
                .iter()
                .find(|entry| {
                    entry.graph_id == graph_id
                        && entry.schema_version == schema_version
                        && entry.registry_version == registry_version
                })
                .map(|entry| entry.key.clone())
        }) else {
            inner.stats.misses = inner.stats.misses.saturating_add(1);
            return None;
        };
        match inner.plans.get(&key) {
            Some(plan) => {
                let plan = Arc::clone(plan);
                inner.stats.hits = inner.stats.hits.saturating_add(1);
                Some(plan)
            }
            None => {
                remove_source_entry(
                    &mut inner,
                    source,
                    graph_id,
                    schema_version,
                    registry_version,
                );
                inner.stats.misses = inner.stats.misses.saturating_add(1);
                None
            }
        }
    }

    pub(crate) fn get(&self, key: &CallPlanKey) -> Option<Arc<ExecutionPlan>> {
        let mut inner = self.lock_inner();
        match inner.plans.get(key) {
            Some(plan) => {
                let plan = Arc::clone(plan);
                inner.stats.hits = inner.stats.hits.saturating_add(1);
                Some(plan)
            }
            None => {
                inner.stats.misses = inner.stats.misses.saturating_add(1);
                None
            }
        }
    }

    pub(crate) fn insert_with_source(
        &self,
        key: CallPlanKey,
        source: Arc<str>,
        plan: Arc<ExecutionPlan>,
    ) {
        self.insert_inner(key, Some(source), plan);
    }

    fn insert_inner(&self, key: CallPlanKey, source: Option<Arc<str>>, plan: Arc<ExecutionPlan>) {
        let mut inner = self.lock_inner();
        let replacing_existing = inner.plans.contains(&key);
        if inner.plans.push(key.clone(), plan).is_some() && !replacing_existing {
            inner.stats.capacity_evictions = inner.stats.capacity_evictions.saturating_add(1);
        }
        if let Some(source) = source {
            let entry = CallPlanSourceEntry {
                graph_id: key.graph_id,
                schema_version: key.schema_version,
                registry_version: key.registry_version,
                key,
            };
            match inner.source_index.get_mut(source.as_ref()) {
                Some(entries) => {
                    if let Some(existing) = entries.iter_mut().find(|existing| {
                        existing.graph_id == entry.graph_id
                            && existing.schema_version == entry.schema_version
                            && existing.registry_version == entry.registry_version
                    }) {
                        *existing = entry;
                    } else {
                        entries.push(entry);
                    }
                }
                None => {
                    inner.source_index.push(source, vec![entry]);
                }
            }
        }
    }

    /// Return a snapshot of the cache counters.
    #[must_use]
    pub fn stats(&self) -> CallPlanCacheStats {
        self.lock_inner().stats
    }

    /// Remove all cached plans while preserving accumulated counters.
    pub fn clear(&self) {
        let mut inner = self.lock_inner();
        inner.plans.clear();
        inner.source_index.clear();
    }

    fn lock_inner(&self) -> MutexGuard<'_, CallPlanCacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn remove_source_entry(
    inner: &mut CallPlanCacheInner,
    source: &str,
    graph_id: GraphId,
    schema_version: u64,
    registry_version: u64,
) {
    let Some(entries) = inner.source_index.get_mut(source) else {
        return;
    };
    entries.retain(|entry| {
        !(entry.graph_id == graph_id
            && entry.schema_version == schema_version
            && entry.registry_version == registry_version)
    });
    if entries.is_empty() {
        inner.source_index.pop(source);
    }
}

impl CallPlanKey {
    pub(crate) fn for_statement(
        graph_id: GraphId,
        schema_version: u64,
        registry_version: u64,
        statement: &Statement,
    ) -> Option<Self> {
        let canonical_source = canonical_call_source(statement)?;
        Some(Self {
            graph_id,
            schema_version,
            registry_version,
            canonical_source: Arc::from(canonical_source),
        })
    }

    /// Return the graph identity carried by this cache key.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// Return the schema-version epoch carried by this cache key.
    #[must_use]
    pub const fn schema_version(&self) -> u64 {
        self.schema_version
    }

    /// Return the procedure-registry epoch carried by this cache key.
    #[must_use]
    pub const fn registry_version(&self) -> u64 {
        self.registry_version
    }

    /// Return the canonical CALL source carried by this cache key.
    #[must_use]
    pub fn canonical_source(&self) -> &str {
        &self.canonical_source
    }
}

fn canonical_call_source(statement: &Statement) -> Option<String> {
    match statement {
        Statement::Call(call) => format_procedure_call(call).ok(),
        Statement::Query(pipeline) if is_call_rooted_pipeline(statement, pipeline) => {
            format_read_statement(statement).ok()
        }
        _ => None,
    }
}

fn is_call_rooted_pipeline(statement: &Statement, pipeline: &crate::QueryPipeline) -> bool {
    let statements = pipeline.statements.as_slice();
    match statements {
        [PipelineStatement::Call(_)]
        | [PipelineStatement::Call(_), PipelineStatement::Return(_)] => {
            format_read_statement(statement).is_ok()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc};

    use selene_core::GraphId;

    use super::*;
    use crate::{
        EmptyProcedureRegistry, ExecutionPlan, analyze, ast::format_procedure_call, parser::parse,
        plan,
    };

    fn key(source: &str) -> CallPlanKey {
        key_with_registry(source, 11)
    }

    fn key_with_registry(source: &str, registry_version: u64) -> CallPlanKey {
        let statement = parse(source).expect("source parses");
        CallPlanKey::for_statement(GraphId::new(7), 3, registry_version, &statement)
            .expect("source produces CALL cache key")
    }

    fn plan_for(source: &str) -> Arc<ExecutionPlan> {
        let statement = parse(source).expect("source parses");
        let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("source analyzes");
        Arc::new(plan(&analyzed, &EmptyProcedureRegistry).expect("source plans"))
    }

    #[test]
    fn call_plan_cache_keys_arg_shape_and_yield_distinctly() {
        let arg_shape = key("CALL cache.echo(1 + 2) YIELD out");
        let arg_value = key("CALL cache.echo(3) YIELD out");
        let yield_order = key("CALL cache.echo() YIELD a, b");
        let yield_order_reversed = key("CALL cache.echo() YIELD b, a");
        let yield_alias = key("CALL cache.echo() YIELD out AS alias");

        assert_ne!(arg_shape, arg_value);
        assert_ne!(yield_order, yield_order_reversed);
        assert_ne!(key("CALL cache.echo() YIELD out"), yield_alias);

        let statement =
            parse("CALL cache.echo(1 + 2, $p) YIELD out AS alias").expect("source parses");
        let Statement::Call(call) = statement else {
            panic!("expected top-level CALL");
        };
        let formatted = format_procedure_call(&call).expect("procedure call formats");
        assert_eq!(formatted, "CALL cache.echo((1 + 2), $p) YIELD out AS alias");
    }

    #[test]
    fn call_plan_key_canonicalizes_whitespace() {
        let compact = key("CALL cache.echo(1+2) YIELD out");
        let spaced = key("CALL cache.echo(1 + 2) YIELD out");

        assert_eq!(compact, spaced);
        assert_eq!(
            compact.canonical_source(),
            "CALL cache.echo((1 + 2)) YIELD out"
        );
    }

    #[test]
    fn embedded_pipeline_call_is_not_keyed() {
        let statement =
            parse("MATCH (n) CALL cache.echo(n) YIELD out RETURN out").expect("source parses");

        assert!(CallPlanKey::for_statement(GraphId::new(7), 3, 11, &statement).is_none());
    }

    #[test]
    fn key_carries_graph_id_schema_version_and_registry_version() {
        let statement = parse("CALL cache.echo()").expect("source parses");
        let graph_one = CallPlanKey::for_statement(GraphId::new(1), 0, 11, &statement)
            .expect("source produces key");
        let graph_two = CallPlanKey::for_statement(GraphId::new(2), 0, 11, &statement)
            .expect("source produces key");
        let schema_one = CallPlanKey::for_statement(GraphId::new(1), 1, 11, &statement)
            .expect("source produces key");
        let registry_one = CallPlanKey::for_statement(GraphId::new(1), 0, 12, &statement)
            .expect("source produces key");

        assert_ne!(graph_one, graph_two);
        assert_ne!(graph_one, schema_one);
        assert_ne!(graph_one, registry_one);
        assert_eq!(graph_one.graph_id(), GraphId::new(1));
        assert_eq!(graph_one.schema_version(), 0);
        assert_eq!(graph_one.registry_version(), 11);
    }

    #[test]
    fn call_plan_cache_tracks_hits_misses_and_evictions() {
        let cache = CallPlanCache::new(NonZeroUsize::new(1).expect("nonzero"));
        let first_key = key("CALL cache.one()");
        let second_key = key("CALL cache.two()");

        assert!(cache.get(&first_key).is_none());
        cache.insert_with_source(
            first_key.clone(),
            Arc::from("CALL cache.one()"),
            plan_for("RETURN 1"),
        );
        assert!(cache.get(&first_key).is_some());
        cache.insert_with_source(
            second_key,
            Arc::from("CALL cache.two()"),
            plan_for("RETURN 2"),
        );
        assert!(cache.get(&first_key).is_none());

        assert_eq!(
            cache.stats(),
            CallPlanCacheStats {
                hits: 1,
                misses: 2,
                capacity_evictions: 1,
            }
        );
    }

    #[test]
    fn call_plan_cache_source_fast_path_hits_existing_plan() {
        let cache = CallPlanCache::new(NonZeroUsize::new(2).expect("nonzero"));
        let source = Arc::<str>::from("CALL cache.one()");
        let key = key(&source);

        cache.insert_with_source(key, Arc::clone(&source), plan_for("RETURN 1"));

        assert!(
            cache
                .get_source(GraphId::new(7), 3, 11, "CALL cache.one()")
                .is_some()
        );
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn call_plan_cache_source_misses_are_recorded() {
        let cache = CallPlanCache::new(NonZeroUsize::new(2).expect("nonzero"));
        let source = Arc::<str>::from("CALL cache.one()");
        let key = key(&source);

        assert!(
            cache
                .get_source(GraphId::new(7), 3, 11, "CALL cache.one()")
                .is_none()
        );
        cache.insert_with_source(key, Arc::clone(&source), plan_for("RETURN 1"));
        assert!(
            cache
                .get_source(GraphId::new(7), 3, 12, "CALL cache.one()")
                .is_none()
        );

        assert_eq!(cache.stats().misses, 2);
    }

    #[test]
    fn call_plan_cache_stale_source_entries_are_recorded_as_misses() {
        let cache = CallPlanCache::new(NonZeroUsize::new(1).expect("nonzero"));
        let source = Arc::<str>::from("CALL cache.one()");
        let old_key = key_with_registry(&source, 11);
        let new_key = key_with_registry(&source, 12);

        cache.insert_with_source(old_key, Arc::clone(&source), plan_for("RETURN 1"));
        cache.insert_with_source(new_key, Arc::clone(&source), plan_for("RETURN 2"));

        assert!(
            cache
                .get_source(GraphId::new(7), 3, 11, "CALL cache.one()")
                .is_none()
        );
        assert!(
            cache
                .get_source(GraphId::new(7), 3, 12, "CALL cache.one()")
                .is_some()
        );

        assert_eq!(
            cache.stats(),
            CallPlanCacheStats {
                hits: 1,
                misses: 1,
                capacity_evictions: 1,
            }
        );
    }
}
