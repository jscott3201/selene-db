//! Shared execution-plan cache for procedure-call-rooted statements.

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex, MutexGuard},
};

use lru::LruCache;
use selene_core::GraphId;
use selene_profile::ProfileIdentity;

use crate::{
    ExecutionPlan, ImplDefinedCaps, PipelineStatement,
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
    profile_identity: ProfileIdentity,
    caps: ImplDefinedCaps,
    index_selection: bool,
    canonical_source: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CallPlanSourceEntry {
    graph_id: GraphId,
    schema_version: u64,
    registry_version: u64,
    profile_identity: ProfileIdentity,
    caps: ImplDefinedCaps,
    index_selection: bool,
    key: CallPlanKey,
}

#[derive(Clone, Copy)]
pub(crate) struct CallPlanSourceLookup<'a> {
    pub(crate) graph_id: GraphId,
    pub(crate) schema_version: u64,
    pub(crate) registry_version: u64,
    pub(crate) profile_identity: ProfileIdentity,
    pub(crate) caps: ImplDefinedCaps,
    pub(crate) index_selection: bool,
    pub(crate) source: &'a str,
}

impl CallPlanSourceEntry {
    fn matches(&self, lookup: CallPlanSourceLookup<'_>) -> bool {
        self.graph_id == lookup.graph_id
            && self.schema_version == lookup.schema_version
            && self.registry_version == lookup.registry_version
            && self.profile_identity == lookup.profile_identity
            && self.caps == lookup.caps
            && self.index_selection == lookup.index_selection
    }
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
        lookup: CallPlanSourceLookup<'_>,
    ) -> Option<Arc<ExecutionPlan>> {
        let mut inner = self.lock_inner();
        let Some(key) = inner.source_index.get(lookup.source).and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.matches(lookup))
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
                remove_source_entry(&mut inner, lookup);
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
                profile_identity: key.profile_identity,
                caps: key.caps,
                index_selection: key.index_selection,
                key,
            };
            match inner.source_index.get_mut(source.as_ref()) {
                Some(entries) => {
                    if let Some(existing) = entries.iter_mut().find(|existing| {
                        existing.graph_id == entry.graph_id
                            && existing.schema_version == entry.schema_version
                            && existing.registry_version == entry.registry_version
                            && existing.profile_identity == entry.profile_identity
                            && existing.caps == entry.caps
                            && existing.index_selection == entry.index_selection
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

fn remove_source_entry(inner: &mut CallPlanCacheInner, lookup: CallPlanSourceLookup<'_>) {
    let Some(entries) = inner.source_index.get_mut(lookup.source) else {
        return;
    };
    entries.retain(|entry| !entry.matches(lookup));
    if entries.is_empty() {
        inner.source_index.pop(lookup.source);
    }
}

impl CallPlanKey {
    pub(crate) fn for_statement(
        graph_id: GraphId,
        schema_version: u64,
        registry_version: u64,
        profile_identity: ProfileIdentity,
        caps: ImplDefinedCaps,
        index_selection: bool,
        statement: &Statement,
    ) -> Option<Self> {
        let canonical_source = canonical_call_source(statement)?;
        Some(Self {
            graph_id,
            schema_version,
            registry_version,
            profile_identity,
            caps,
            index_selection,
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

    /// Return the generated profile identity carried by this cache key.
    #[must_use]
    pub const fn profile_identity(&self) -> ProfileIdentity {
        self.profile_identity
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
        // A CALL-rooted query pipeline canonicalizes through the read-side
        // formatter. The structural test is allocation-free; the statement is
        // formatted exactly once (here), with `.ok()` propagating a format
        // failure as a cache miss.
        Statement::Query(pipeline) if is_call_rooted_pipeline(pipeline) => {
            format_read_statement(statement).ok()
        }
        _ => None,
    }
}

fn is_call_rooted_pipeline(pipeline: &crate::QueryPipeline) -> bool {
    matches!(
        pipeline.statements.as_slice(),
        [PipelineStatement::Call(_)] | [PipelineStatement::Call(_), PipelineStatement::Return(_)]
    )
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc};

    use selene_core::GraphId;
    use selene_profile::{ProfileIdentity, current_profile_identity};

    use super::*;
    use crate::{
        EmptyProcedureRegistry, ExecutionPlan, analyze, ast::format_procedure_call, parser::parse,
        plan,
    };

    fn key(source: &str) -> CallPlanKey {
        key_with_registry(source, 11)
    }

    fn key_with_registry(source: &str, registry_version: u64) -> CallPlanKey {
        key_with_profile(source, registry_version, current_profile_identity())
    }

    fn key_with_profile(
        source: &str,
        registry_version: u64,
        profile_identity: ProfileIdentity,
    ) -> CallPlanKey {
        let statement = parse(source).expect("source parses");
        CallPlanKey::for_statement(
            GraphId::new(7),
            3,
            registry_version,
            profile_identity,
            ImplDefinedCaps::DEFAULT,
            true,
            &statement,
        )
        .expect("source produces CALL cache key")
    }

    fn source_lookup(
        source: &str,
        registry_version: u64,
        profile_identity: ProfileIdentity,
    ) -> CallPlanSourceLookup<'_> {
        CallPlanSourceLookup {
            graph_id: GraphId::new(7),
            schema_version: 3,
            registry_version,
            profile_identity,
            caps: ImplDefinedCaps::DEFAULT,
            index_selection: true,
            source,
        }
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
        assert_ne!(
            key("CALL cache.echo($p)"),
            key("CALL cache.echo($p :: INT)")
        );
        assert_ne!(
            key("CALL cache.echo($p :: INT)"),
            key("CALL cache.echo($p :: STRING)")
        );
        assert_eq!(
            key("CALL cache.echo($p :: INT)").canonical_source(),
            "CALL cache.echo($p :: INTEGER)"
        );

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

        assert!(
            CallPlanKey::for_statement(
                GraphId::new(7),
                3,
                11,
                current_profile_identity(),
                ImplDefinedCaps::DEFAULT,
                true,
                &statement,
            )
            .is_none()
        );
    }

    #[test]
    fn key_carries_graph_id_schema_version_and_registry_version() {
        let statement = parse("CALL cache.echo()").expect("source parses");
        let make = |graph_id, schema_version, registry_version, profile_identity, caps, indexes| {
            CallPlanKey::for_statement(
                graph_id,
                schema_version,
                registry_version,
                profile_identity,
                caps,
                indexes,
                &statement,
            )
            .expect("source produces key")
        };
        let graph_one = make(
            GraphId::new(1),
            0,
            11,
            current_profile_identity(),
            ImplDefinedCaps::DEFAULT,
            true,
        );
        let graph_two = make(
            GraphId::new(2),
            0,
            11,
            current_profile_identity(),
            ImplDefinedCaps::DEFAULT,
            true,
        );
        let schema_one = make(
            GraphId::new(1),
            1,
            11,
            current_profile_identity(),
            ImplDefinedCaps::DEFAULT,
            true,
        );
        let registry_one = make(
            GraphId::new(1),
            0,
            12,
            current_profile_identity(),
            ImplDefinedCaps::DEFAULT,
            true,
        );
        let profile_one = make(
            GraphId::new(1),
            0,
            11,
            ProfileIdentity::new("synthetic", 3, 3, "other-hash"),
            ImplDefinedCaps::DEFAULT,
            true,
        );
        let caps_one = make(
            GraphId::new(1),
            0,
            11,
            current_profile_identity(),
            ImplDefinedCaps::DEFAULT.with_max_list_length(1),
            true,
        );
        let no_indexes = make(
            GraphId::new(1),
            0,
            11,
            current_profile_identity(),
            ImplDefinedCaps::DEFAULT,
            false,
        );

        assert_ne!(graph_one, graph_two);
        assert_ne!(graph_one, schema_one);
        assert_ne!(graph_one, registry_one);
        assert_ne!(graph_one, profile_one);
        assert_ne!(graph_one, caps_one);
        assert_ne!(graph_one, no_indexes);
        assert_eq!(graph_one.graph_id(), GraphId::new(1));
        assert_eq!(graph_one.schema_version(), 0);
        assert_eq!(graph_one.registry_version(), 11);
        assert_eq!(graph_one.profile_identity(), current_profile_identity());
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
                .get_source(source_lookup(
                    "CALL cache.one()",
                    11,
                    current_profile_identity(),
                ))
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
                .get_source(source_lookup(
                    "CALL cache.one()",
                    11,
                    current_profile_identity(),
                ))
                .is_none()
        );
        cache.insert_with_source(key, Arc::clone(&source), plan_for("RETURN 1"));
        assert!(
            cache
                .get_source(source_lookup(
                    "CALL cache.one()",
                    12,
                    current_profile_identity(),
                ))
                .is_none()
        );

        assert_eq!(cache.stats().misses, 2);
    }

    #[test]
    fn call_plan_cache_stale_source_entries_are_recorded_as_misses() {
        let cache = CallPlanCache::new(NonZeroUsize::new(1).expect("nonzero"));
        let source = Arc::<str>::from("CALL cache.one()");
        let old_profile = current_profile_identity();
        let new_profile = ProfileIdentity::new("synthetic", 3, 3, "other-hash");
        let old_key = key_with_profile(&source, 11, old_profile);
        let new_key = key_with_profile(&source, 11, new_profile);

        cache.insert_with_source(old_key, Arc::clone(&source), plan_for("RETURN 1"));
        cache.insert_with_source(new_key, Arc::clone(&source), plan_for("RETURN 2"));

        assert!(
            cache
                .get_source(source_lookup("CALL cache.one()", 11, old_profile))
                .is_none()
        );
        assert!(
            cache
                .get_source(source_lookup("CALL cache.one()", 11, new_profile))
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
