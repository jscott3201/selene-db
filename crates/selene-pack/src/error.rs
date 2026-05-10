//! Registry construction errors.

use selene_core::IStr;
use selene_gql::ProcedureTier;

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
}
