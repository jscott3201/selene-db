//! Catalog ownership types for Selene DB.
//!
//! This lower crate is an advanced engine boundary, not part of the stable 2.x
//! embedding API. Applications should depend on `selene-db` instead.
//!
//! [`BootstrapCatalog`] remains the temporary identity used to place one
//! in-memory graph behind the facade. The catalog-owned descriptor and immutable
//! read-snapshot model is separate from that bridge. M02-PR03 owns lifecycle and
//! writer publication; M02-PR05 deletes the bootstrap after named graphs work.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod descriptor;
mod error;
mod identity;
mod name;
mod snapshot;
mod transaction;

pub use descriptor::{
    CatalogDescriptor, CatalogParent, CatalogPayload, CoreGraphTypeBridge, CreationMetadata,
};
pub use error::{CatalogError, CatalogResult};
pub use identity::{
    BindingTableId, CatalogGeneration, CatalogId, CatalogObjectId, CatalogObjectKind, ConstraintId,
    DirectoryId, GraphId, GraphTypeId, IndexId, ProcedureId, SchemaId,
};
pub use name::{CATALOG_UNICODE_VERSION, CatalogName, IdentifierForm};
pub use snapshot::{CatalogMemoryAccounting, CatalogSnapshot, CatalogSnapshotBuilder};
pub use transaction::CatalogTransaction;

use selene_core::GraphId as CoreGraphId;
use selene_profile::{ProfileIdentity, current_profile_identity};

const BOOTSTRAP_GRAPH_ID: CoreGraphId = CoreGraphId::new(1);
const DEFAULT_CATALOG_NAME: &str = "selene";
const DEFAULT_SCHEMA_NAME: &str = "public";
const DEFAULT_GRAPH_NAME: &str = "default";

/// Temporary identity for the facade's single in-memory graph.
///
/// The facade keeps this value private. It exists in this crate so later M02
/// work can replace the bootstrap without making graph storage a catalog
/// dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapCatalog {
    graph_id: CoreGraphId,
    profile: ProfileIdentity,
}

impl BootstrapCatalog {
    /// Construct the bootstrap identity for the generated runtime profile.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            graph_id: BOOTSTRAP_GRAPH_ID,
            profile: current_profile_identity(),
        }
    }

    /// Return the internal graph identity used to construct graph storage.
    #[must_use]
    pub const fn graph_id(self) -> CoreGraphId {
        self.graph_id
    }

    /// Return the generated profile identity bound to this bootstrap.
    #[must_use]
    pub const fn profile(self) -> ProfileIdentity {
        self.profile
    }

    /// Return the temporary default catalog name.
    #[must_use]
    pub const fn catalog_name(self) -> &'static str {
        DEFAULT_CATALOG_NAME
    }

    /// Return the temporary default schema name.
    #[must_use]
    pub const fn schema_name(self) -> &'static str {
        DEFAULT_SCHEMA_NAME
    }

    /// Return the temporary default graph name.
    #[must_use]
    pub const fn graph_name(self) -> &'static str {
        DEFAULT_GRAPH_NAME
    }
}

impl Default for BootstrapCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_identity_is_non_tombstone_and_profile_bound() {
        let catalog = BootstrapCatalog::new();

        assert_ne!(catalog.graph_id(), CoreGraphId::TOMBSTONE);
        assert_eq!(catalog.profile(), current_profile_identity());
        assert_eq!(catalog.catalog_name(), "selene");
        assert_eq!(catalog.schema_name(), "public");
        assert_eq!(catalog.graph_name(), "default");
    }
}
