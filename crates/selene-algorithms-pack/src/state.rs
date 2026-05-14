//! Shared pack state.

use std::collections::HashMap;

use parking_lot::Mutex;
use selene_algorithms::{AlgorithmsError, GraphProjection, ProjectionCatalog};
use selene_core::GraphId;
use selene_gql::{GraphContext, ProcedureError};

use crate::error::algorithm_error;

/// Engine-lifetime algorithms-pack state.
#[derive(Debug, Default)]
pub(crate) struct AlgorithmsPackState {
    catalogs: Mutex<HashMap<GraphId, ProjectionCatalog>>,
}

impl AlgorithmsPackState {
    pub(crate) fn with_catalog<R>(
        &self,
        graph_id: GraphId,
        f: impl FnOnce(&ProjectionCatalog) -> Result<R, ProcedureError>,
    ) -> Result<R, ProcedureError> {
        let mut catalogs = self.catalogs.lock();
        let catalog = catalogs.entry(graph_id).or_default();
        f(catalog)
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
