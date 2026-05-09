//! Shared test fixtures and corpus loaders for selene-db.
//!
//! See spec 10 section 6 for the corpus contract. `selene-testing` is
//! internal (`publish = false`) and is consumed by integration tests.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod analyzed_corpus;
pub mod closed_graph_fixtures;
pub mod corpus;
pub mod mock_index_catalog;
pub mod mock_procedure_registry;

pub use closed_graph_fixtures::{person_company_graph_type, person_only_graph_type};
pub use mock_index_catalog::MockIndexCatalog;
pub use mock_procedure_registry::{MockProcedureRegistry, default_corpus_registry};
