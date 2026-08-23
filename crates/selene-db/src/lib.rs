//! Stable 2.x embedding facade for Selene DB.
//!
//! Applications should depend on this crate rather than assembling engine
//! layers. Lower crates remain available for advanced engine work, but they do
//! not carry this crate's 2.x stability promise unless a type is intentionally
//! re-exported here.
//!
//! The current facade owns one in-memory bootstrap graph. Named catalogs,
//! schemas, graphs, persistence, parameters, transaction state, and row-value
//! materialization are not exposed yet. M02-PR05 deletes the bootstrap adapter
//! after catalog-backed named graphs are operational. Results are summaries in
//! this skeleton; they do not expose engine binding tables or physical rows.
//!
//! # Quickstart
//!
//! ```
//! use selene_db::{Database, ExecutionOutcome, WriteSummary};
//!
//! let database = Database::builder().build();
//! let session = database.session();
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

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod config;
mod database;
mod error;
mod outcome;
mod session;

pub use config::{DatabaseConfig, OpenMode};
pub use database::{Database, DatabaseBuilder};
pub use error::{Error, ErrorKind, GqlStatus};
pub use outcome::{ExecutionOutcome, WriteSummary};
pub use session::Session;

/// Result type returned by facade operations.
pub type Result<T> = std::result::Result<T, Error>;
