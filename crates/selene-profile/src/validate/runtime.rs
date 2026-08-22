//! Runtime compatibility-list validation.

use std::collections::BTreeSet;

use crate::{ClaimState, Profile, RuntimeSupport};

use super::{ProfileError, invalid, require_text};

pub(super) fn validate(profile: &Profile) -> Result<(), ProfileError> {
    let mut ids = BTreeSet::new();
    let mut supported = BTreeSet::new();
    let mut orders = BTreeSet::new();
    for feature in &profile.features {
        if !ids.insert(feature.id.as_str()) {
            return Err(invalid(format!(
                "duplicate runtime ID {}",
                feature.id.as_str()
            )));
        }
        validate_entry(
            feature.id.as_str(),
            feature.runtime_support,
            feature.claim_state,
            &feature.unsupported_rationale,
        )?;
        if feature.runtime_support == RuntimeSupport::Supported {
            supported.insert(feature.id.as_str());
        }
        if !orders.insert(feature.runtime_order) {
            return Err(invalid(format!(
                "duplicate runtime_order {}",
                feature.runtime_order
            )));
        }
    }
    for extension in &profile.implementation_extensions {
        if !ids.insert(extension.id.as_str()) {
            return Err(invalid(format!(
                "duplicate runtime ID {}",
                extension.id.as_str()
            )));
        }
        let state = if extension.runtime_support == RuntimeSupport::Supported {
            ClaimState::ImplementedUnclaimed
        } else {
            ClaimState::Unsupported
        };
        validate_entry(
            extension.id.as_str(),
            extension.runtime_support,
            state,
            &extension.unsupported_rationale,
        )?;
        if extension.runtime_support == RuntimeSupport::Supported {
            supported.insert(extension.id.as_str());
        }
        if !orders.insert(extension.runtime_order) {
            return Err(invalid(format!(
                "duplicate runtime_order {}",
                extension.runtime_order
            )));
        }
    }
    let mut choice_orders = BTreeSet::new();
    for choice in &profile.implementation_defined_choices {
        if !choice_orders.insert(choice.runtime_order) {
            return Err(invalid(format!(
                "duplicate implementation-defined runtime_order {}",
                choice.runtime_order
            )));
        }
    }
    for id in &profile.supported_feature_order {
        if !ids.contains(id.as_str()) {
            return Err(invalid(format!(
                "supported_feature_order references unknown runtime ID {}",
                id.as_str()
            )));
        }
    }
    let ordered = profile
        .supported_feature_order
        .iter()
        .map(|id| id.as_str())
        .collect::<BTreeSet<_>>();
    if supported != ordered {
        return Err(invalid(
            "supported_feature_order must contain every runtime-supported feature and extension exactly once",
        ));
    }
    Ok(())
}

fn validate_entry(
    id: &str,
    support: RuntimeSupport,
    claim: ClaimState,
    rationale: &str,
) -> Result<(), ProfileError> {
    if support == RuntimeSupport::Unsupported {
        require_text("unsupported rationale", rationale)?;
    } else if !rationale.is_empty() {
        return Err(invalid(format!(
            "{id} has a rationale but is not runtime-unsupported"
        )));
    }
    if (support == RuntimeSupport::Supported && claim == ClaimState::Unsupported)
        || (support != RuntimeSupport::Supported && claim != ClaimState::Unsupported)
    {
        Err(invalid(format!(
            "{id} has incompatible runtime support {support:?} and claim state {claim:?}"
        )))
    } else {
        Ok(())
    }
}
