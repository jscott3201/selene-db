//! Database ownership and construction.

use std::sync::Arc;

use selene_catalog::BootstrapCatalog;
use selene_gql::BuiltinProcedureRegistry;
use selene_graph::SharedGraph;

use crate::{DatabaseConfig, Session};

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
    pub(crate) bootstrap: BootstrapState,
    pub(crate) procedures: BuiltinProcedureRegistry,
}

impl DatabaseInner {
    fn new(config: DatabaseConfig) -> Self {
        let catalog = BootstrapCatalog::new();
        Self {
            config,
            bootstrap: BootstrapState {
                graph: SharedGraph::new(catalog.graph_id()),
                catalog,
            },
            procedures: BuiltinProcedureRegistry::new(),
        }
    }
}

pub(crate) struct BootstrapState {
    pub(crate) catalog: BootstrapCatalog,
    pub(crate) graph: SharedGraph,
}
