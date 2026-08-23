//! Annex B completeness and typed-decision validation.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DecisionStability, ImplementationDefinedDecision, ImplementationDefinedValue, Profile,
    inventory,
};

use super::{ProfileError, invalid, require_text};

const MAX_TOPIC_CHARS: usize = 96;
const MAX_EXPLANATION_CHARS: usize = 320;
const MAX_VALUE_CHARS: usize = 256;
const MAX_LIST_VALUES: usize = 64;

const KNOWN_PENDING_OWNERS: &[&str] = &[
    "M01-PR04", "M02-PR01", "M02-PR02", "M03-PR01", "M03-PR02", "M03-PR03", "M03-PR04", "M03-PR05",
    "M05-PR01", "M05-PR03", "M05-PR06", "M06-PR05", "M07-PR03", "M08-PR02", "M10-PR04",
];

pub(super) fn validate_inventory(profile: &Profile) -> Result<(), ProfileError> {
    let actual = profile
        .implementation_defined_choices
        .iter()
        .map(|record| record.id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = inventory::CATEGORIES
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect::<BTreeSet<_>>();
    if expected.len() != inventory::TOTAL {
        return Err(invalid("internal Annex B inventory count is not 117"));
    }
    if actual != expected {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let extra = actual.difference(&expected).copied().collect::<Vec<_>>();
        return Err(invalid(format!(
            "Annex B inventory mismatch; missing [{}]; extra [{}]",
            missing.join(", "),
            extra.join(", ")
        )));
    }
    for (category, ids) in inventory::CATEGORIES {
        let count = actual.iter().filter(|id| id.starts_with(category)).count();
        if count != ids.len() {
            return Err(invalid(format!(
                "Annex B category {category} must contain {}, got {count}",
                ids.len()
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_decisions(
    profile: &Profile,
    applicability: &BTreeMap<String, bool>,
) -> Result<(), ProfileError> {
    for record in &profile.implementation_defined_choices {
        bounded_text(
            "implementation-defined topic",
            &record.topic,
            MAX_TOPIC_CHARS,
        )?;
        if record.clause_anchors.is_empty() {
            return Err(invalid(format!(
                "{} must cite at least one clause anchor",
                record.id.as_str()
            )));
        }
        if record.evidence.is_empty() {
            return Err(invalid(format!(
                "{} must cite at least one evidence reference",
                record.id.as_str()
            )));
        }
        let applies = applicability[record.applicability.as_str()];
        match &record.decision {
            ImplementationDefinedDecision::Selected {
                value,
                rationale,
                stability,
                ..
            } => {
                if !applies {
                    return Err(disposition_mismatch(record.id.as_str(), false, "selected"));
                }
                validate_value(value)?;
                bounded_text("selected rationale", rationale, MAX_EXPLANATION_CHARS)?;
                if profile.release_claimable && *stability == DecisionStability::Provisional {
                    return Err(invalid(format!(
                        "release-claimable profile has provisional decision {}",
                        record.id.as_str()
                    )));
                }
            }
            ImplementationDefinedDecision::Pending { owner, reason } => {
                if !applies {
                    return Err(disposition_mismatch(record.id.as_str(), false, "pending"));
                }
                validate_owner(owner)?;
                bounded_text("pending reason", reason, MAX_EXPLANATION_CHARS)?;
                if profile.release_claimable {
                    return Err(invalid(format!(
                        "release-claimable profile has pending decision {}",
                        record.id.as_str()
                    )));
                }
            }
            ImplementationDefinedDecision::NotApplicable { reason } => {
                if applies {
                    return Err(disposition_mismatch(
                        record.id.as_str(),
                        true,
                        "not_applicable",
                    ));
                }
                bounded_text("not-applicable reason", reason, MAX_EXPLANATION_CHARS)?;
            }
        }
    }
    Ok(())
}

fn disposition_mismatch(id: &str, applies: bool, disposition: &str) -> ProfileError {
    invalid(format!(
        "{id} applicability is {applies} but disposition is {disposition}"
    ))
}

fn validate_value(value: &ImplementationDefinedValue) -> Result<(), ProfileError> {
    match value {
        ImplementationDefinedValue::Boolean { .. }
        | ImplementationDefinedValue::UnsignedInteger { .. } => Ok(()),
        ImplementationDefinedValue::Identifier { value } => {
            bounded_text("selected identifier", value, MAX_VALUE_CHARS)
        }
        ImplementationDefinedValue::String { value } => {
            bounded_text("selected string", value, MAX_VALUE_CHARS)
        }
        ImplementationDefinedValue::OrderedIdentifierList { value } => {
            validate_list("selected identifier list", value)
        }
        ImplementationDefinedValue::OrderedStringList { value } => {
            validate_list("selected string list", value)
        }
    }
}

fn validate_list(kind: &str, values: &[String]) -> Result<(), ProfileError> {
    if values.is_empty() || values.len() > MAX_LIST_VALUES {
        return Err(invalid(format!(
            "{kind} must contain 1..={MAX_LIST_VALUES} values"
        )));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        bounded_text(kind, value, MAX_VALUE_CHARS)?;
        if !seen.insert(value) {
            return Err(invalid(format!(
                "{kind} contains duplicate value {value:?}"
            )));
        }
    }
    Ok(())
}

fn validate_owner(owner: &str) -> Result<(), ProfileError> {
    if !KNOWN_PENDING_OWNERS.contains(&owner) {
        return Err(invalid(format!(
            "pending owner {owner:?} is not a known bounded work item"
        )));
    }
    Ok(())
}

fn bounded_text(kind: &str, text: &str, max_chars: usize) -> Result<(), ProfileError> {
    require_text(kind, text)?;
    if text.chars().count() > max_chars {
        return Err(invalid(format!("{kind} exceeds {max_chars} characters")));
    }
    let lower = text.to_ascii_lowercase();
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| matches!(word, "tbd" | "todo" | "placeholder" | "unknown"))
    {
        return Err(invalid(format!("{kind} contains placeholder text")));
    }
    Ok(())
}
