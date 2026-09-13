//! Canonical GQL profile records, validation, and checked-in generated data.
//!
//! Runtime consumers use the generated constants. Profile JSON is loaded only
//! by the explicit generator and profile tooling.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod closure;
mod conformance;
mod generate;
mod generated;
mod inventory;
mod model;
mod runtime;
mod validate;

pub use conformance::{
    EvidenceDisposition, EvidenceExpectation, EvidenceRecord, EvidenceSource, EvidenceTarget,
    ExpectedNullability, ExpectedOrder, ExpectedSideEffects, ExpectedStatus, ExpectedType,
    FeatureScope, InventoryState, RequirementKind, RuleApplicability, RuleRecord, RuleRequirement,
    RulesSource, ValidatedConformance, load_conformance, parse_conformance,
};
pub use generate::{check_repository, render_outputs, write_repository};
pub use generated::{
    ANNEX_B_CATEGORY_COUNTS, ANNEX_B_IA, ANNEX_B_ID, ANNEX_B_IE, ANNEX_B_IL, ANNEX_B_IS,
    ANNEX_B_IV, ANNEX_B_IW, ANNEX_B_LOOKUP_TEST_VECTORS, DIRECT_SELECTED_FEATURES,
    PROFILE_FORMAT_VERSION, PROFILE_GENERATOR_VERSION, PROFILE_HASH, PROFILE_ID, RELEASE_CLAIMABLE,
    TARGET_FEATURE_CLOSURE, annex_b_by_id, annex_b_records,
};
pub use model::{
    ApplicabilityDefinition, ApplicabilityExpression, ApplicabilityId, ClaimState, ClauseAnchor,
    ClauseAnchorId, CompatibilityId, DecisionStability, DecisionVisibility, EvidenceId,
    EvidenceReference, ExtensionId, FeatureCode, FeatureRecord, ImplDefinedId, ImplDependentId,
    ImplementationDefinedChoiceRecord, ImplementationDefinedDecision, ImplementationDefinedValue,
    ImplementationDependentNote, ImplementationExtension, Implication, ImplicationId, Profile,
    RuntimeSupport,
};
pub use runtime::{
    AnnexBDecision, AnnexBId, AnnexBRecord, AnnexBValue, ApplicabilityStatus, CapabilityClaimState,
    CapabilityRecord, CapabilityStatus, DecisionStability as RuntimeDecisionStability,
    DecisionVisibility as RuntimeDecisionVisibility, EvidenceStatus, FeatureId, FeatureSurface,
    FixedTimeZoneDisplacement, FlaggerStatus, ProfileIdentity, ProfileRelation, SessionDefaults,
    SessionUserDeclaredType,
};
pub use validate::{ProfileError, ValidatedProfile, load_profile, parse_profile};

/// Return the generated identity used by runtime compilation and cache paths.
#[must_use]
pub const fn current_profile_identity() -> ProfileIdentity {
    generated::PROFILE_IDENTITY
}

/// Return the typed defaults copied into each new facade session context.
#[must_use]
pub const fn current_session_defaults() -> SessionDefaults {
    generated::SESSION_DEFAULTS
}

/// Return every ISO feature and extension in deterministic runtime order.
#[must_use]
pub const fn capabilities() -> &'static [CapabilityRecord] {
    generated::CAPABILITY_RECORDS
}

/// Look up one generated capability by typed identifier.
#[must_use]
pub fn capability(id: FeatureId) -> Option<&'static CapabilityRecord> {
    capability_by_id(id.as_str())
}

/// Look up one generated capability by stable identifier text.
#[must_use]
pub fn capability_by_id(id: &str) -> Option<&'static CapabilityRecord> {
    generated::capability_by_id(id)
}
