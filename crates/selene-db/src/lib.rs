//! Stable 2.x embedding facade for Selene DB.
//!
//! Applications should depend on this crate rather than assembling engine
//! layers. Lower crates remain available for advanced engine work, but they do
//! not carry this crate's 2.x stability promise unless a type is intentionally
//! re-exported here.
//!
//! The current facade owns one in-memory catalog with named schemas, graphs, and
//! closed graph types. A [`Session`] owns an immutable [`SessionContext`] with
//! copied catalog/profile defaults and optional embedder-provided authorization.
//! Parameter mutation, request state, transactions, termination transitions,
//! persistence, and row-value materialization are deferred.
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
//! let session = database.session(&graph_path)?;
//!
//! let write = session.execute("INSERT (:Person { name: 'Ada' })")?;
//! assert_eq!(
//!     write,
//!     ExecutionOutcome::Written(WriteSummary::new(1, None)),
//! );
//!
//! let rows = session.execute("MATCH (n:Person) RETURN n")?;
//! assert_eq!(rows, ExecutionOutcome::Rows { row_count: 1 });
//! # Ok::<(), selene_db::Error>(())
//! ```
//!
//! Removed graph handles and lower engine types are not facade exports:
//!
//! ```compile_fail
//! use selene_db::{GraphHandle, SharedGraph};
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
//! use selene_db::{CoreGraphTypeBridge, CoreProvider, GraphTypeDef, SeleneGraph};
//! ```
//!
//! The facade session has no borrowed graph lifetime:
//!
//! ```compile_fail
//! fn borrowed(_: selene_db::Session<'static>) {}
//! ```
//!
//! A session is movable between threads but is intentionally not shareable for
//! concurrent use:
//!
//! ```compile_fail
//! fn require_sync<T: Sync>() {}
//! require_sync::<selene_db::Session>();
//! ```
//!
//! Session context fields cannot be overwritten through the public API:
//!
//! ```compile_fail
//! fn overwrite(
//!     context: &mut selene_db::SessionContext,
//!     graph: selene_db::GraphDescriptor,
//! ) {
//!     context.current_graph = graph;
//! }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod auth;
mod catalog;
mod catalog_snapshot;
mod config;
mod database;
mod ddl;
mod error;
mod graph_type;
mod outcome;
mod path;
mod session;
mod session_context;

pub use auth::{
    AllowAllAuthorizationPolicy, AuthHookError, AuthorizationDecision, AuthorizationId,
    AuthorizationPolicy, AuthorizationRequest, NoPrincipalProvider, Principal, PrincipalId,
    PrincipalProvider, SessionOptions,
};
pub use catalog::{Catalog, CreateOutcome, CreatePolicy, DropOutcome, DropPolicy};
pub use catalog_snapshot::{
    CatalogGeneration, CatalogReadSnapshot, GraphDescriptor, GraphId, GraphTypeDescriptor,
    GraphTypeId, SchemaDescriptor, SchemaId,
};
pub use config::{DatabaseConfig, OpenMode};
pub use database::{Database, DatabaseBuilder};
pub use error::{Error, ErrorKind, GqlStatus};
pub use graph_type::{GraphTypeBuilder, GraphTypeDefinition, NodeTypeDefinition};
pub use outcome::{ExecutionOutcome, WriteSummary};
pub use path::{CatalogPath, ObjectPath, PathSegment, SchemaPath};
pub use session::Session;
pub use session_context::{
    ProfileIdentity, RequestSlotState, SessionContext, SessionDependencySummary, SessionParameters,
    SessionTerminationState, TimeZoneDisplacement, TransactionSlotState,
};

/// Result type returned by facade operations.
pub type Result<T> = std::result::Result<T, Error>;
