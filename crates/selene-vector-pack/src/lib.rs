//! Procedure-pack adapters for `selene-vector`.
//!
//! The crate exposes read-tier vector index operations through GQL `CALL` by
//! registering an external pack with `selene-pack` at registry construction
//! time.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod error;
mod provider;
mod registry;
mod search;
mod state;

pub use registry::{VECTOR_PACK_NAME, VECTOR_PROCEDURE_NAMES, VectorPack};
pub use state::VectorPackState;
