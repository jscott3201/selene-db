//! Compatibility adapter over the generated `selene-profile` registry.
//!
//! Keep runtime consumers on this path until M01-PR04 removes the adapter.

pub use selene_profile::{
    ANNEX_B_REGISTER, AnnexBDecision, AnnexBId, AnnexBRecord, AnnexBRegister, AnnexBValue,
    ApplicabilityStatus, FLAGGER_ACCEPTED_FEATURES, FeatureId, NOT_SUPPORTED_RATIONALE,
    REFERENCED_FEATURES, RuntimeDecisionStability, RuntimeDecisionVisibility, SUPPORTED_FEATURES,
};

/// True when `id` is in the existing runtime-supported feature set.
#[must_use]
pub fn is_supported(id: FeatureId) -> bool {
    SUPPORTED_FEATURES.contains(&id)
}

/// True when the compatibility flagger accepts syntax carrying `id`.
///
/// M01-PR04 owns removal of this compatibility distinction when flagging moves
/// to generated profile policy.
#[must_use]
pub fn is_flagger_accepted(id: FeatureId) -> bool {
    FLAGGER_ACCEPTED_FEATURES.contains(&id)
}

/// Return the display name for a referenced feature or extension ID.
#[must_use]
pub fn name_of(id: FeatureId) -> Option<&'static str> {
    REFERENCED_FEATURES
        .iter()
        .find_map(|(feature, name)| (*feature == id).then_some(*name))
}

/// Return a referenced ID from its stable string representation.
#[must_use]
pub fn feature_id_from_str(id: &str) -> Option<FeatureId> {
    REFERENCED_FEATURES
        .iter()
        .find_map(|(feature, _)| (feature.as_str() == id).then_some(*feature))
}

/// Return the exact runtime non-support rationale for an ID.
#[must_use]
pub fn non_supported_rationale(id: FeatureId) -> Option<&'static str> {
    NOT_SUPPORTED_RATIONALE
        .iter()
        .find_map(|(feature, rationale)| (*feature == id).then_some(*rationale))
}

#[cfg(test)]
mod tests;
