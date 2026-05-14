//! Procedure-pack adapters for `selene-algorithms`.
//!
//! The crate exposes graph algorithms through GQL `CALL` by registering an
//! external pack with `selene-pack` at registry construction time.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod betweenness;
mod community;
mod error;
mod pagerank;
mod pathfinding;
mod projection;
mod registry;
mod state;
mod structural;

pub use pagerank::{DEFAULT_DAMPING, DEFAULT_MAX_ITERATIONS, DEFAULT_TOLERANCE};
pub use registry::{ALGO_PROCEDURE_NAMES, ALGORITHMS_PACK_NAME, AlgorithmsPack};
