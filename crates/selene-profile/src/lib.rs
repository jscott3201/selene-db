//! Canonical GQL profile records, validation, and checked-in generated data.
//!
//! Runtime consumers use the generated constants. Profile JSON is loaded only
//! by the explicit generator and profile tooling.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod closure;
mod generate;
mod generated;
mod model;
mod runtime;
mod validate;

pub use generate::{check_repository, render_outputs, write_repository};
pub use generated::{
    ANNEX_B_REGISTER, DIRECT_SELECTED_FEATURES, FLAGGER_ACCEPTED_FEATURES, NOT_SUPPORTED_RATIONALE,
    PROFILE_FORMAT_VERSION, PROFILE_GENERATOR_VERSION, PROFILE_HASH, PROFILE_ID,
    REFERENCED_FEATURES, RELEASE_CLAIMABLE, SUPPORTED_FEATURES, TARGET_FEATURE_CLOSURE,
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
