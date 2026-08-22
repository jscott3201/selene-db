//! Canonical GQL profile records, validation, and checked-in generated data.
//!
//! Runtime consumers use the generated constants. Profile JSON is loaded only
//! by the explicit generator and profile tooling.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod generate;
mod generated;
mod model;
mod runtime;
mod validate;

pub use generate::{check_repository, render_outputs, write_repository};
pub use generated::{
    ANNEX_B_REGISTER, NOT_SUPPORTED_RATIONALE, PROFILE_FORMAT_VERSION, PROFILE_HASH,
    REFERENCED_FEATURES, SUPPORTED_FEATURES,
};
pub use model::{
    ApplicabilityDefinition, ApplicabilityExpression, ApplicabilityId, ClaimState, ClauseAnchor,
    ClauseAnchorId, CompatibilityId, EvidenceId, EvidenceReference, ExtensionId, FeatureCode,
    FeatureRecord, ImplDefinedId, ImplDependentId, ImplementationDefinedChoiceRecord,
    ImplementationDependentNote, ImplementationExtension, Implication, ImplicationId, Profile,
    RuntimeSupport,
};
pub use runtime::{AnnexBId, FeatureId, ImplDefinedChoice};
pub use validate::{ProfileError, ValidatedProfile, load_profile, parse_profile};
