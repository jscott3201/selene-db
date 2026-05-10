//! Built-in procedure pack registry for selene-db.
//!
//! `selene-pack` owns concrete built-in registration and implements the
//! procedure-registry boundary consumed by `selene-gql`. Registration is
//! construct-time-only in v1.0; callers receive a frozen registry that supports
//! plan-time lookup and runtime dispatch.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod builtin;
mod error;
mod registry;

pub use error::RegistryError;
pub use registry::{ProcedurePackRegistry, ProcedurePackRegistryBuilder};
pub use selene_core::IStr;
pub use selene_gql::{
    GraphContext, MutationContext, ProcedureContext, ProcedureError, ProcedureHandle,
    ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema,
    ProcedureParameter, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    Value,
};
