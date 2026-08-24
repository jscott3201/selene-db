//! Catalog ownership types for Selene DB.
//!
//! This lower crate is an advanced engine boundary, not part of the stable 2.x
//! embedding API. Applications should depend on `selene-db` instead.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod descriptor;
mod error;
mod identity;
mod name;
mod snapshot;
mod transaction;

pub use descriptor::{CatalogDescriptor, CatalogParent, CatalogPayload, CreationMetadata};
pub use error::{CatalogError, CatalogResult};
pub use identity::{
    BindingTableId, CatalogGeneration, CatalogId, CatalogObjectId, CatalogObjectKind, ConstraintId,
    DirectoryId, GraphId, GraphTypeId, IndexId, ProcedureId, SchemaId,
};
pub use name::{CATALOG_UNICODE_VERSION, CatalogName, IdentifierForm};
pub use snapshot::{CatalogMemoryAccounting, CatalogSnapshot, CatalogSnapshotBuilder};
pub use transaction::CatalogTransaction;
