//! Registry construction errors.

use selene_core::IStr;
use selene_gql::{ProcedureMutability, ProcedureTier};

/// Failure while building a frozen procedure-pack registry.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// A procedure name was registered with two different content hashes.
    #[error("procedure registration conflict for {name:?}")]
    Conflict {
        /// Canonical procedure name.
        name: Box<[IStr]>,
        /// Hash associated with the first registration.
        existing_hash: [u8; 32],
        /// Hash associated with the attempted registration.
        new_hash: [u8; 32],
    },
    /// Two external packs used the same pack name during registry build.
    #[error("external procedure-pack registration conflict for {name:?}")]
    PackConflict {
        /// Static external pack name.
        name: String,
    },
    /// An external pack name was malformed.
    #[error("invalid external procedure-pack name {name:?}: {reason}")]
    InvalidPackName {
        /// Static external pack name.
        name: String,
        /// Stable reason tag.
        reason: &'static str,
    },
    /// A tier-typed builder method received metadata declaring another tier.
    #[error("procedure tier mismatch for {name:?}: declared {declared:?}, attempted {attempted:?}")]
    TierMismatch {
        /// Canonical procedure name.
        name: Box<[IStr]>,
        /// Tier declared by the built-in metadata.
        declared: ProcedureTier,
        /// Tier implied by the builder method.
        attempted: ProcedureTier,
    },
    /// A procedure name was malformed.
    #[error("invalid procedure name {name:?}: {reason}")]
    InvalidName {
        /// Raw string segments supplied by the built-in.
        name: Box<[String]>,
        /// Stable reason tag.
        reason: &'static str,
    },
    /// Persist-tier registration is intentionally deferred out of v1.0.
    #[error("persist-tier procedure registration is not implemented in v1.0 for {name:?}")]
    PersistTierNotInV1 {
        /// Canonical procedure name.
        name: Box<[IStr]>,
    },
    /// The global string interner refused a built-in metadata string.
    #[error("procedure registry exhausted interner capacity while interning {detail}")]
    InternerCapExhausted {
        /// Stable detail naming the static metadata string.
        detail: String,
    },
    /// The built-in's declared tier is inconsistent with its declared mutability.
    ///
    /// Mirrors `selene_gql::runtime::pipeline::call::context::validate_call_tier`'s
    /// `tier_for_mutability` mapping: `Read` ↔ `Graph` (or `Persist`); any write
    /// mutability ↔ `Mutation`. Surfacing this at build prevents a runtime
    /// `ProcedureError::TierMismatch` after planning succeeds.
    #[error(
        "procedure mutability/tier inconsistency for {name:?}: declared tier {declared_tier:?} \
         with mutability {declared_mutability:?}, but mutability implies tier {expected_tier:?}"
    )]
    MutabilityTierMismatch {
        /// Canonical procedure name.
        name: Box<[IStr]>,
        /// Tier declared by the built-in metadata.
        declared_tier: ProcedureTier,
        /// Mutability declared by the built-in metadata.
        declared_mutability: ProcedureMutability,
        /// Tier implied by the declared mutability.
        expected_tier: ProcedureTier,
    },
}
