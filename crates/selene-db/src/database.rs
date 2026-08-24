//! Database ownership, immutable outer state, and construction.

use std::{collections::BTreeMap, sync::Arc};

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};
use selene_catalog::{
    CatalogDescriptor, CatalogGeneration, CatalogId, CatalogName, CatalogSnapshot,
    CatalogSnapshotBuilder, CreationMetadata, DirectoryId, GraphId, GraphTypeId,
};
use selene_gql::BuiltinProcedureRegistry;
use selene_graph::{GraphTypeDef, SharedGraph};

use crate::{Catalog, DatabaseConfig, Error, ObjectPath, Result, Session};

const CATALOG_NAME: &str = "selene";

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

    /// Open a movable, lifetime-free session selected to the graph at `path`.
    ///
    /// The session records the current stable graph identity. Dropping or
    /// replacing that graph invalidates the session, including when the same
    /// path is recreated later.
    ///
    /// # Errors
    ///
    /// Returns a structured catalog error when the path, its parents, or its
    /// object kind do not identify a current graph.
    pub fn session(&self, path: &ObjectPath) -> Result<Session> {
        let descriptor = self.catalog().snapshot().resolve_graph(path)?;
        let id = GraphId::new(descriptor.id.get()).map_err(Error::from_catalog_invariant)?;
        Ok(Session::new(Arc::clone(&self.inner), id, descriptor.path))
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
    /// The current builder has no fallible configuration or I/O path.
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
    #[cfg(test)]
    pub(crate) failure: Mutex<Option<crate::catalog::FailurePoint>>,
    #[cfg(test)]
    pub(crate) drop_blocked: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl DatabaseInner {
    /// Acquire the lifecycle writer mutex.
    ///
    /// Every catalog lifecycle mutation starts here. In test builds it also
    /// asserts that the calling thread holds no graph request lease: drop
    /// takes the target's lifecycle write lease after this mutex, so entering
    /// under a same-thread read lease would deadlock instead of failing.
    pub(crate) fn lock_lifecycle_writer(&self) -> parking_lot::MutexGuard<'_, ()> {
        #[cfg(test)]
        assert_eq!(
            GraphRequestDepth::current(),
            0,
            "catalog lifecycle entered under a same-thread graph request lease"
        );
        self.lifecycle_writer.lock()
    }

    fn new(config: DatabaseConfig) -> Self {
        Self {
            config,
            state: ArcSwap::from(Arc::new(DatabaseState {
                catalog: initial_snapshot(),
                graphs: BTreeMap::new(),
                graph_types: BTreeMap::new(),
                high_water: HighWaterMarks::initial(),
            })),
            lifecycle_writer: Mutex::new(()),
            procedures: BuiltinProcedureRegistry::new(),
            #[cfg(test)]
            failure: Mutex::new(None),
            #[cfg(test)]
            drop_blocked: Mutex::new(None),
        }
    }
}

/// Test-only per-thread count of active graph request leases.
#[cfg(test)]
pub(crate) struct GraphRequestDepth;

#[cfg(test)]
impl GraphRequestDepth {
    thread_local! {
        static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    pub(crate) fn enter() -> Self {
        Self::DEPTH.with(|depth| depth.set(depth.get() + 1));
        Self
    }

    pub(crate) fn current() -> usize {
        Self::DEPTH.with(std::cell::Cell::get)
    }
}

#[cfg(test)]
impl Drop for GraphRequestDepth {
    fn drop(&mut self) {
        Self::DEPTH.with(|depth| depth.set(depth.get() - 1));
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
    const fn initial() -> Self {
        Self {
            schema: 0,
            graph: 0,
            graph_type: 0,
        }
    }
}

fn initial_snapshot() -> CatalogSnapshot {
    let generation = CatalogGeneration::new(1).expect("initial generation is nonzero");
    let creation = CreationMetadata::new(generation, None);
    let catalog_id = CatalogId::new(1).expect("catalog ID is nonzero");
    let root_id = DirectoryId::new(1).expect("directory ID is nonzero");
    let catalog = CatalogDescriptor::catalog(
        catalog_id,
        CatalogName::regular(CATALOG_NAME).expect("catalog name is valid"),
        generation,
        creation.clone(),
    )
    .expect("catalog descriptor is valid");
    let root = CatalogDescriptor::root_directory(root_id, catalog_id, generation, creation.clone())
        .expect("root descriptor is valid");
    CatalogSnapshotBuilder::new(generation, catalog, root)
        .expect("catalog and root are related")
        .build()
        .expect("initial catalog snapshot is complete")
}
