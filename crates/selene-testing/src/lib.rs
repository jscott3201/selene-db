//! Shared test fixtures and corpus loaders for selene-db.
//!
//! See spec 10 section 6 for the corpus contract. `selene-testing` is
//! internal (`publish = false`) and is consumed by integration tests.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod analyzed_corpus;
pub mod bench_fixtures;
pub mod bench_profiles;
pub mod closed_graph_fixtures;
pub mod corpus;
pub mod executor_corpus;
pub mod mock_index_catalog;
pub mod mock_procedure_registry;
pub mod plan_corpus;

pub use bench_fixtures::BenchFixture;
pub use bench_profiles::BenchProfile;
pub use closed_graph_fixtures::{person_company_graph_type, person_only_graph_type};
pub use executor_corpus::{
    ExecutorCorpus, ExecutorCorpusCategory, ExecutorCorpusEntry, ExecutorCorpusFixture,
    ExecutorCorpusProgram, ExecutorCorpusRegistry, ExecutorOperator, PHASE_A_OPERATORS,
};
pub use mock_index_catalog::MockIndexCatalog;
pub use mock_procedure_registry::{MockProcedureRegistry, default_corpus_registry};
pub use plan_corpus::{PlanCorpus, PlanCorpusCategory, PlanCorpusEntry, PlanCorpusRegistry};
