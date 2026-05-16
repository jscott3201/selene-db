//! Shared pack state.

use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;
use selene_algorithms::{AlgorithmsError, GraphProjection, ProjectionCatalog};
use selene_core::GraphId;
use selene_gql::{GraphContext, ProcedureError};

use crate::error::algorithm_error;

/// Engine-lifetime algorithms-pack state.
#[derive(Debug, Default)]
pub(crate) struct AlgorithmsPackState {
    catalogs: RwLock<HashMap<GraphId, Arc<ProjectionCatalog>>>,
}

impl AlgorithmsPackState {
    pub(crate) fn with_catalog<R>(
        &self,
        graph_id: GraphId,
        f: impl FnOnce(&ProjectionCatalog) -> Result<R, ProcedureError>,
    ) -> Result<R, ProcedureError> {
        if let Some(catalog) = self.catalogs.read().get(&graph_id).cloned() {
            return f(catalog.as_ref());
        }

        let catalog = {
            let mut catalogs = self.catalogs.write();
            Arc::clone(
                catalogs
                    .entry(graph_id)
                    .or_insert_with(|| Arc::new(ProjectionCatalog::new())),
            )
        };
        f(catalog.as_ref())
    }
}

pub(crate) fn with_algorithm_projection<R>(
    state: &AlgorithmsPackState,
    ctx: &GraphContext<'_>,
    projection_name: &str,
    f: impl FnOnce(&GraphProjection) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let graph_id = ctx.snapshot().graph_id();
    state.with_catalog(graph_id, |catalog| {
        catalog
            .ensure_fresh(ctx.snapshot(), projection_name)
            .map_err(algorithm_error)?;
        let projection = catalog.get(projection_name).ok_or_else(|| {
            algorithm_error(AlgorithmsError::NoSuchProjection {
                name: projection_name.to_owned(),
            })
        })?;
        f(projection.projection())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    use selene_core::GraphId;

    use super::*;

    #[test]
    fn with_catalog_runs_concurrently_against_different_graph_ids() {
        let state = Arc::new(AlgorithmsPackState::default());
        let active = Arc::new(AtomicUsize::new(0));
        let max_observed = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(4));

        let mut handles = Vec::new();
        for raw_id in 1..=4 {
            let state = Arc::clone(&state);
            let active = Arc::clone(&active);
            let max_observed = Arc::clone(&max_observed);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                state
                    .with_catalog(GraphId::new(raw_id), |_catalog| {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_observed.fetch_max(now, Ordering::SeqCst);
                        barrier.wait();
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .expect("with_catalog closure succeeds");
            }));
        }

        for handle in handles {
            handle.join().expect("thread should not panic");
        }

        let observed = max_observed.load(Ordering::SeqCst);
        assert!(
            observed >= 2,
            "expected at least 2 with_catalog closures executing concurrently against different \
             GraphIds, observed max = {observed}",
        );
    }
}
