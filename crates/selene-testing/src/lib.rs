//! Shared test fixtures and corpus loaders for selene-db.
//!
//! See spec 10 section 6 for the corpus contract. `selene-testing` is
//! internal (`publish = false`) and is consumed by integration tests.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod analyzed_corpus;
pub mod corpus;
