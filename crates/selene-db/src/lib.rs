//! Stable 2.x embedding facade for Selene DB.
//!
//! Applications should depend on this crate rather than assembling engine
//! layers. Lower crates remain available for advanced engine work, but they do
//! not carry this crate's 2.x stability promise unless a type is intentionally
//! re-exported here.
//!
//! The current facade owns one in-memory catalog with named schemas, graphs, and
//! closed graph types. Persistence, parameters, transaction state, and row-value
//! materialization are not exposed yet. The compatibility [`Session`] still
//! targets `/selene/public/default`; M02-PR05 removes that bootstrap bridge.
//!
//! # Quickstart
//!
//! ```
//! use selene_db::{
//!     CreatePolicy, Database, ExecutionOutcome, ObjectPath, SchemaPath, WriteSummary,
//! };
//!
//! let database = Database::builder().build();
//! let catalog = database.catalog();
//! let schema = SchemaPath::regular("selene", "memory")?;
//! catalog.create_schema(&schema, CreatePolicy::Strict)?;
//! let graph_path = ObjectPath::regular("selene", "memory", "episodes")?;
//! catalog.create_graph(&graph_path, None, CreatePolicy::Strict)?;
//! let graph = catalog.open_graph(&graph_path)?;
//!
//! let write = graph.execute("INSERT (:Person { name: 'Ada' })")?;
//! assert_eq!(
//!     write,
//!     ExecutionOutcome::Written(WriteSummary::new(1, None)),
//! );
//!
//! let rows = graph.execute("MATCH (n:Person) RETURN n")?;
//! assert_eq!(rows, ExecutionOutcome::Rows { row_count: 1 });
//! # Ok::<(), selene_db::Error>(())
//! ```
//!
//! Engine graph handles are not facade exports:
//!
//! ```compile_fail
//! use selene_db::SharedGraph;
//! ```
//!
//! Physical row indices are not facade exports:
//!
//! ```compile_fail
//! use selene_db::RowIndex;
//! ```
//!
//! Lower mutation builders are not facade exports:
//!
//! ```compile_fail
//! use selene_db::Mutator;
//! ```
//!
//! Persistence writers are not facade exports:
//!
//! ```compile_fail
//! use selene_db::WalWriter;
//! ```
//!
//! Lower runtime graph and schema definitions are not facade exports:
//!
//! ```compile_fail
//! use selene_db::{CoreGraphTypeBridge, GraphTypeDef, SeleneGraph};
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod catalog;
mod catalog_snapshot;
mod config;
mod database;
mod error;
mod graph_handle;
mod graph_type;
mod outcome;
mod path;
mod session;

pub use catalog::{Catalog, CreateOutcome, CreatePolicy, DropOutcome, DropPolicy};
pub use catalog_snapshot::{
    CatalogGeneration, CatalogReadSnapshot, GraphDescriptor, GraphId, GraphTypeDescriptor,
    GraphTypeId, SchemaDescriptor, SchemaId,
};
pub use config::{DatabaseConfig, OpenMode};
pub use database::{Database, DatabaseBuilder};
pub use error::{Error, ErrorKind, GqlStatus};
pub use graph_handle::GraphHandle;
pub use graph_type::{GraphTypeBuilder, GraphTypeDefinition, NodeTypeDefinition};
pub use outcome::{ExecutionOutcome, WriteSummary};
pub use path::{CatalogPath, ObjectPath, PathSegment, SchemaPath};
pub use session::Session;

/// Result type returned by facade operations.
pub type Result<T> = std::result::Result<T, Error>;
