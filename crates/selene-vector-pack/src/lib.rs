//! Procedure-pack adapters for `selene-vector`.
//!
//! The crate exposes read-tier vector index operations through GQL `CALL` by
//! registering an external pack with `selene-pack` at registry construction
//! time.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod bulk_delete;
mod bulk_upsert;
mod delete;
mod error;
mod ivf_bulk_delete;
mod ivf_bulk_upsert;
mod provider;
mod registry;
mod search;
mod state;
mod upsert;

pub use registry::{VECTOR_PACK_NAME, VECTOR_PROCEDURE_NAMES, VectorPack};
pub use state::VectorPackState;
