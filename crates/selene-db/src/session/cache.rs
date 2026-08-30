//! Private selected-facade dependency stamp and one-entry plan cache.

use selene_catalog::{CatalogObjectId, GraphId as LowerGraphId, GraphTypeId as LowerGraphTypeId};
use selene_gql::{PreparedCatalogPlan, ProcedureRegistry};

use crate::{
    CatalogReadSnapshot, Error, GqlType, ProfileIdentity, Result, catalog_snapshot,
    session::Session,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequestPlanKey {
    pub(crate) source: String,
    pub(crate) parameter_types: Vec<(String, GqlType)>,
}

/// Complete temporary facade dependency stamp.
///
/// M04-PR01 replaces this private identity/generation bridge with final public
/// catalog reference types. M05-PR02 replaces its semantic-site assumptions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DependencyStamp {
    pub(crate) catalog_generation: u64,
    pub(crate) schema: (u64, u64),
    pub(crate) graph: (u64, u64, u64, u64),
    pub(crate) graph_type: Option<(u64, u64)>,
    pub(crate) procedure_registry_version: u64,
    pub(crate) profile: ProfileStamp,
    pub(crate) characteristic_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProfileStamp {
    profile_id: String,
    source_format_version: u32,
    generator_version: u32,
    canonical_hash: String,
}

impl From<&ProfileIdentity> for ProfileStamp {
    fn from(profile: &ProfileIdentity) -> Self {
        Self {
            profile_id: profile.profile_id().to_owned(),
            source_format_version: profile.source_format_version(),
            generator_version: profile.generator_version(),
            canonical_hash: profile.canonical_hash().to_owned(),
        }
    }
}

struct CacheEntry {
    key: RequestPlanKey,
    stamp: DependencyStamp,
    plan: PreparedCatalogPlan,
}

#[derive(Default)]
pub(crate) struct SessionPlanCache {
    entry: Option<CacheEntry>,
    #[cfg(test)]
    hits: u64,
    #[cfg(test)]
    misses: u64,
}

impl SessionPlanCache {
    pub(crate) fn lookup(
        &mut self,
        key: &RequestPlanKey,
        stamp: &DependencyStamp,
    ) -> Option<PreparedCatalogPlan> {
        let matched = self
            .entry
            .as_ref()
            .filter(|entry| entry.key == *key && entry.stamp == *stamp)
            .map(|entry| entry.plan.clone());
        #[cfg(test)]
        if matched.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        matched
    }

    pub(crate) fn insert(
        &mut self,
        key: RequestPlanKey,
        stamp: DependencyStamp,
        plan: PreparedCatalogPlan,
    ) {
        self.entry = Some(CacheEntry { key, stamp, plan });
    }

    pub(crate) fn clear(&mut self) {
        self.entry = None;
    }

    #[cfg(test)]
    pub(crate) const fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

impl Session {
    pub(super) fn capture_dependency_stamp(&self) -> Result<DependencyStamp> {
        let snapshot = CatalogReadSnapshot {
            state: self.inner.state.load_full(),
        };
        self.validate_context_snapshot(&snapshot)?;
        let schema = self.context.current_schema();
        let graph = self.context.current_graph();
        let graph_type = graph
            .graph_type
            .map(|id| {
                let lower = LowerGraphTypeId::new(id.get()).map_err(|source| {
                    Error::invalid_session_reference(Error::from_catalog_invariant(source))
                })?;
                let descriptor = snapshot
                    .state
                    .catalog
                    .descriptor(CatalogObjectId::GraphType(lower))
                    .ok_or_else(Error::stale_session_reference)?;
                let summary = catalog_snapshot::graph_type_summary(&snapshot.state, descriptor)?;
                Ok((summary.id.get(), summary.created_at.get()))
            })
            .transpose()?;
        let lower_graph = LowerGraphId::new(graph.id.get()).map_err(|source| {
            Error::invalid_session_reference(Error::from_catalog_invariant(source))
        })?;
        let (runtime_generation, schema_version) =
            self.inner
                .with_graph_request(lower_graph, &graph.path, |runtime| {
                    let runtime_generation = runtime.read().meta.generation;
                    Ok((runtime_generation, runtime.schema_version()))
                })?;
        Ok(DependencyStamp {
            catalog_generation: snapshot.generation().get(),
            schema: (schema.id.get(), schema.created_at.get()),
            graph: (
                graph.id.get(),
                graph.created_at.get(),
                runtime_generation,
                schema_version,
            ),
            graph_type,
            procedure_registry_version: self.inner.procedures.registry_version(),
            profile: ProfileStamp::from(self.context.profile_identity()),
            characteristic_epoch: self.context.characteristic_epoch(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> DependencyStamp {
        let profile = ProfileStamp::from(&ProfileIdentity::current());
        DependencyStamp {
            catalog_generation: 1,
            schema: (2, 3),
            graph: (4, 5, 6, 7),
            graph_type: Some((8, 9)),
            procedure_registry_version: 10,
            profile,
            characteristic_epoch: 11,
        }
    }

    #[test]
    fn every_execution_dependency_participates_in_equality() {
        let base = stamp();
        let mut variants = Vec::new();
        let mut changed = base.clone();
        changed.catalog_generation += 1;
        variants.push(changed);
        let mut changed = base.clone();
        changed.schema.0 += 1;
        variants.push(changed);
        let mut changed = base.clone();
        changed.schema.1 += 1;
        variants.push(changed);
        for index in 0..4 {
            let mut changed = base.clone();
            let graph = &mut changed.graph;
            match index {
                0 => graph.0 += 1,
                1 => graph.1 += 1,
                2 => graph.2 += 1,
                _ => graph.3 += 1,
            }
            variants.push(changed);
        }
        let mut changed = base.clone();
        changed.graph_type = None;
        variants.push(changed);
        let mut changed = base.clone();
        changed.graph_type.as_mut().unwrap().1 += 1;
        variants.push(changed);
        let mut changed = base.clone();
        changed.procedure_registry_version += 1;
        variants.push(changed);
        let mut changed = base.clone();
        changed.profile.canonical_hash.push('0');
        variants.push(changed);
        let mut changed = base.clone();
        changed.characteristic_epoch += 1;
        variants.push(changed);

        assert!(variants.into_iter().all(|changed| changed != base));
    }
}
