//! Database ownership, immutable outer state, and construction.

use std::{collections::BTreeMap, sync::Arc};

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};
use selene_catalog::{
    BootstrapCatalog, CatalogDescriptor, CatalogGeneration, CatalogId, CatalogName,
    CatalogObjectId, CatalogSnapshot, CatalogSnapshotBuilder, CreationMetadata, DirectoryId,
    GraphId, GraphTypeId, SchemaId,
};
use selene_gql::BuiltinProcedureRegistry;
use selene_graph::{GraphTypeDef, SharedGraph};

use crate::{Catalog, DatabaseConfig, Session};

pub(crate) fn bootstrap_schema_id() -> SchemaId {
    SchemaId::new(1).expect("bootstrap schema ID is nonzero")
}

pub(crate) fn bootstrap_graph_id() -> GraphId {
    GraphId::new(1).expect("bootstrap graph ID is nonzero")
}

/// Embedded database ownership root.
///
/// Cloning a database clones its internal [`Arc`]. Sessions clone the same
/// ownership root and remain usable after every `Database` handle is dropped.
#[derive(Clone)]
pub struct Database {
    inner: Arc<DatabaseInner>,
}

impl Database {
    /// Start an in-memory database build with the default configuration.
    #[must_use]
    pub fn builder() -> DatabaseBuilder {
        DatabaseBuilder::new()
    }

    /// Open a movable, lifetime-free session over this database.
    #[must_use]
    pub fn session(&self) -> Session {
        Session::new(Arc::clone(&self.inner))
    }

    /// Open the database-owned catalog lifecycle service.
    #[must_use]
    pub fn catalog(&self) -> Catalog {
        Catalog::new(Arc::clone(&self.inner))
    }

    /// Borrow the configuration used to build this database.
    #[must_use]
    pub fn config(&self) -> &DatabaseConfig {
        &self.inner.config
    }
}

/// Construction funnel for [`Database`].
#[derive(Clone, Debug, Default)]
pub struct DatabaseBuilder {
    config: DatabaseConfig,
}

impl DatabaseBuilder {
    /// Create a builder for the supported in-memory mode.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder from an explicit configuration.
    #[must_use]
    pub const fn from_config(config: DatabaseConfig) -> Self {
        Self { config }
    }

    /// Borrow the configuration this builder will consume.
    #[must_use]
    pub const fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// Build a database.
    ///
    /// M02-PR01 has no fallible configuration or I/O path, so construction is
    /// infallible. Persistence work will add its own explicit open result.
    #[must_use]
    pub fn build(self) -> Database {
        Database {
            inner: Arc::new(DatabaseInner::new(self.config)),
        }
    }
}

pub(crate) struct DatabaseInner {
    pub(crate) config: DatabaseConfig,
    pub(crate) state: ArcSwap<DatabaseState>,
    pub(crate) lifecycle_writer: Mutex<()>,
    pub(crate) procedures: BuiltinProcedureRegistry,
    pub(crate) bootstrap: BootstrapCatalog,
    #[cfg(test)]
    pub(crate) failure: Mutex<Option<crate::catalog::FailurePoint>>,
    #[cfg(test)]
    pub(crate) drop_blocked: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl DatabaseInner {
    fn new(config: DatabaseConfig) -> Self {
        let bootstrap = BootstrapCatalog::new();
        let graph = Arc::new(GraphInstance::new(SharedGraph::new(bootstrap.graph_id())));
        let mut graphs = BTreeMap::new();
        graphs.insert(bootstrap_graph_id(), graph);
        Self {
            config,
            state: ArcSwap::from(Arc::new(DatabaseState {
                catalog: bootstrap_snapshot(bootstrap),
                graphs,
                graph_types: BTreeMap::new(),
                high_water: HighWaterMarks::bootstrap(),
            })),
            lifecycle_writer: Mutex::new(()),
            procedures: BuiltinProcedureRegistry::new(),
            bootstrap,
            #[cfg(test)]
            failure: Mutex::new(None),
            #[cfg(test)]
            drop_blocked: Mutex::new(None),
        }
    }
}

pub(crate) struct DatabaseState {
    pub(crate) catalog: CatalogSnapshot,
    pub(crate) graphs: BTreeMap<GraphId, Arc<GraphInstance>>,
    pub(crate) graph_types: BTreeMap<GraphTypeId, Arc<GraphTypeDef>>,
    pub(crate) high_water: HighWaterMarks,
}

pub(crate) struct GraphInstance {
    pub(crate) graph: SharedGraph,
    pub(crate) lifecycle: RwLock<()>,
}

impl GraphInstance {
    pub(crate) fn new(graph: SharedGraph) -> Self {
        Self {
            graph,
            lifecycle: RwLock::new(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HighWaterMarks {
    pub(crate) schema: u64,
    pub(crate) graph: u64,
    pub(crate) graph_type: u64,
}

impl HighWaterMarks {
    const fn bootstrap() -> Self {
        Self {
            schema: 1,
            graph: 1,
            // Every published high-water mark is nonzero. Graph-type ID 1 is
            // the initial lifecycle sentinel and is never exposed as identity.
            graph_type: 1,
        }
    }
}

fn bootstrap_snapshot(bootstrap: BootstrapCatalog) -> CatalogSnapshot {
    let generation = CatalogGeneration::new(1).expect("bootstrap generation is nonzero");
    let creation = CreationMetadata::new(generation, None);
    let catalog_id = CatalogId::new(1).expect("bootstrap catalog ID is nonzero");
    let root_id = DirectoryId::new(1).expect("bootstrap directory ID is nonzero");
    let catalog = CatalogDescriptor::catalog(
        catalog_id,
        CatalogName::regular(bootstrap.catalog_name()).expect("bootstrap catalog name is valid"),
        generation,
        creation.clone(),
    )
    .expect("bootstrap catalog descriptor is valid");
    let root = CatalogDescriptor::root_directory(root_id, catalog_id, generation, creation.clone())
        .expect("bootstrap root descriptor is valid");
    let mut builder = CatalogSnapshotBuilder::new(generation, catalog, root)
        .expect("bootstrap catalog and root are related");
    builder
        .insert(
            CatalogDescriptor::schema(
                bootstrap_schema_id(),
                CatalogName::regular(bootstrap.schema_name())
                    .expect("bootstrap schema name is valid"),
                root_id,
                generation,
                creation.clone(),
            )
            .expect("bootstrap schema descriptor is valid"),
        )
        .expect("bootstrap schema ID is unique");
    builder
        .insert(
            CatalogDescriptor::graph(
                bootstrap_graph_id(),
                CatalogName::regular(bootstrap.graph_name())
                    .expect("bootstrap graph name is valid"),
                bootstrap_schema_id(),
                generation,
                creation,
                None,
            )
            .expect("bootstrap graph descriptor is valid"),
        )
        .expect("bootstrap graph ID is unique");
    let snapshot = builder.build().expect("bootstrap snapshot is complete");
    debug_assert!(
        snapshot
            .descriptor(CatalogObjectId::Graph(bootstrap_graph_id()))
            .is_some()
    );
    snapshot
}
