//! Shared pack state.

use std::collections::HashMap;

use parking_lot::Mutex;
use selene_algorithms::ProjectionCatalog;
use selene_core::GraphId;
use selene_gql::ProcedureError;

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
