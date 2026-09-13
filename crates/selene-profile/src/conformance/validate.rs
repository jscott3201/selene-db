//! Closed decode, canonical hashing, and static cross-registry validation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::conformance::{
    EvidenceDisposition, EvidenceRecord, EvidenceSource, ExpectedStatus, ExpectedType,
    FeatureScope, InventoryState, RequirementKind, RuleApplicability, RuleRecord, RulesSource,
    ValidatedConformance, read_source,
};
use crate::{ProfileError, ValidatedProfile};

const RULES_PATH: &str = "spec/gql-profile/rules.json";
const EVIDENCE_PATH: &str = "spec/gql-profile/evidence.json";
const SOURCE_VERSION: u32 = 1;
const RULES_REGISTRY_VERSION: u32 = 1;
const EVIDENCE_REGISTRY_VERSION: u32 = 2;

/// Load and validate the checked-in static registries.
pub fn load_conformance(
    root: &Path,
    profile: &ValidatedProfile,
) -> Result<ValidatedConformance, ProfileError> {
    parse_conformance(
        &read_source(&root.join(RULES_PATH))?,
        &read_source(&root.join(EVIDENCE_PATH))?,
        profile,
    )
}

/// Decode and validate rule and evidence JSON against one canonical profile.
pub fn parse_conformance(
    rules: &str,
    evidence: &str,
    profile: &ValidatedProfile,
) -> Result<ValidatedConformance, ProfileError> {
    validate_sources(
        serde_json::from_str(rules)?,
        serde_json::from_str(evidence)?,
        profile,
    )
}

fn invalid(message: impl Into<String>) -> ProfileError {
    ProfileError::Invalid(format!("conformance: {}", message.into()))
}

fn validate_sources(
    mut rules: RulesSource,
    mut evidence: EvidenceSource,
    profile: &ValidatedProfile,
) -> Result<ValidatedConformance, ProfileError> {
    validate_rules(&rules, profile)?;
    canonicalize_rules(&mut rules);
    let canonical_rules = serde_json::to_vec(&rules)?;
    let rules_hash = hash(&canonical_rules);
    validate_evidence(&rules, &evidence, profile, &rules_hash)?;
    canonicalize_evidence(&mut evidence);
    let canonical_evidence = serde_json::to_vec(&evidence)?;
    let evidence_hash = hash(&canonical_evidence);
    Ok(ValidatedConformance {
        rules,
        evidence,
        canonical_rules,
        canonical_evidence,
        rules_hash,
        evidence_hash,
    })
}

fn validate_rules(rules: &RulesSource, profile: &ValidatedProfile) -> Result<(), ProfileError> {
    if rules.format_version != SOURCE_VERSION || rules.registry_version != RULES_REGISTRY_VERSION {
        return Err(invalid(
            "rules format_version and registry_version must both be 1",
        ));
    }
    if rules.profile_id != profile.profile().profile_id {
        return Err(invalid("rules profile_id does not match profile.json"));
    }
    let closure = target_closure(profile);
    let FeatureScope::ProfileTargetClosure {
        expected_count,
        feature_ids_hash,
    } = &rules.target;
    let closure_hash = hash(&serde_json::to_vec(&closure)?);
    if *expected_count != closure.len() || feature_ids_hash != &closure_hash {
        return Err(invalid(format!(
            "target closure declaration does not match the canonical profile closure: expected count {} and hash {closure_hash}",
            closure.len()
        )));
    }
    unique(
        "approved domain",
        rules.approved_domains.iter().map(String::as_str),
    )?;
    let domains = rules
        .approved_domains
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if domains.is_empty() {
        return Err(invalid("approved_domains must not be empty"));
    }
    let clauses = profile
        .profile()
        .clause_anchors
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(domain) = domains.iter().find(|domain| !clauses.contains(**domain)) {
        return Err(invalid(format!(
            "approved domain {domain} is not a profile clause anchor"
        )));
    }
    let mut rule_ids = BTreeSet::new();
    for rule in &rules.rules {
        if !rule_ids.insert(rule.id.as_str()) {
            return Err(invalid(format!("duplicate rule ID {}", rule.id)));
        }
        validate_rule(rule, &domains, &clauses, &closure)?;
    }
    if rules.rules.is_empty() {
        return Err(invalid("rule seed is empty"));
    }
    Ok(())
}

fn validate_rule(
    rule: &RuleRecord,
    domains: &BTreeSet<&str>,
    clauses: &BTreeSet<&str>,
    closure: &BTreeSet<String>,
) -> Result<(), ProfileError> {
    if !valid_prefixed(&rule.id, "RULE-") || !valid_text(&rule.label) {
        return Err(invalid(format!("malformed rule record {}", rule.id)));
    }
    unique("rule clause", rule.clause_ids.iter().map(String::as_str))?;
    if rule.clause_ids.is_empty() {
        return Err(invalid(format!("{} has no clause references", rule.id)));
    }
    for clause_id in &rule.clause_ids {
        if !domains.contains(clause_id.as_str()) || !clauses.contains(clause_id.as_str()) {
            return Err(invalid(format!(
                "{} references unknown or unapproved clause {clause_id}",
                rule.id
            )));
        }
    }
    validate_owner(&rule.owner_milestone, &rule.owner_pr, &rule.id)?;
    unique("rule feature", rule.features.iter().map(String::as_str))?;
    for feature in &rule.features {
        if !closure.contains(feature) {
            return Err(invalid(format!(
                "{} references unknown target feature {feature}",
                rule.id
            )));
        }
    }
    if let RuleApplicability::Feature { feature_id } = &rule.applicability
        && (!closure.contains(feature_id) || !rule.features.contains(feature_id))
    {
        return Err(invalid(format!(
            "{} has unknown feature applicability {feature_id}",
            rule.id
        )));
    }
    unique(
        "requirement",
        rule.requirements.iter().map(|item| item.id.as_str()),
    )?;
    if rule.requirements.is_empty() {
        return Err(invalid(format!("{} has no evidence requirements", rule.id)));
    }
    for requirement in &rule.requirements {
        if !valid_prefixed(&requirement.id, "REQ-") {
            return Err(invalid(format!(
                "{} has malformed requirement {}",
                rule.id, requirement.id
            )));
        }
    }
    Ok(())
}

fn validate_evidence(
    rules: &RulesSource,
    evidence: &EvidenceSource,
    profile: &ValidatedProfile,
    rules_hash: &str,
) -> Result<(), ProfileError> {
    if evidence.format_version != SOURCE_VERSION
        || evidence.registry_version != EVIDENCE_REGISTRY_VERSION
    {
        return Err(invalid(
            "evidence format_version must be 1 and registry_version must be 2",
        ));
    }
    if evidence.profile_id != profile.profile().profile_id
        || evidence.profile_hash != profile.hash()
        || evidence.rules_hash != rules_hash
    {
        return Err(invalid(format!(
            "evidence registry has stale profile/rules hash bindings; expected profile {} and rules {rules_hash}",
            profile.hash()
        )));
    }
    let profile_evidence = profile
        .profile()
        .evidence
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let rule_map = rules
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect::<BTreeMap<_, _>>();
    unique(
        "evidence",
        evidence.evidence.iter().map(|item| item.id.as_str()),
    )?;
    let mut registrations = BTreeSet::new();
    let mut target_states = BTreeMap::new();
    for record in &evidence.evidence {
        validate_evidence_record(
            record,
            &profile_evidence,
            &rule_map,
            &mut registrations,
            &mut target_states,
        )?;
    }
    for rule in &rules.rules {
        for requirement in &rule.requirements {
            if !target_states.contains_key(&(rule.id.as_str(), requirement.id.as_str())) {
                return Err(invalid(format!(
                    "{} requirement {} has no evidence record",
                    rule.id, requirement.id
                )));
            }
        }
    }
    if rules.inventory_state == InventoryState::Complete
        && evidence
            .evidence
            .iter()
            .any(|record| matches!(record.disposition, EvidenceDisposition::Pending { .. }))
    {
        return Err(invalid(
            "complete inventory cannot contain pending evidence",
        ));
    }
    Ok(())
}

fn validate_evidence_record<'a>(
    record: &'a EvidenceRecord,
    profile_evidence: &BTreeSet<&str>,
    rule_map: &BTreeMap<&str, &RuleRecord>,
    registrations: &mut BTreeSet<&'a str>,
    target_states: &mut BTreeMap<(&'a str, &'a str), bool>,
) -> Result<(), ProfileError> {
    if !valid_prefixed(&record.id, "EVID-") || !profile_evidence.contains(record.id.as_str()) {
        return Err(invalid(format!(
            "unknown profile evidence ID {}",
            record.id
        )));
    }
    if record.targets.is_empty() {
        return Err(invalid(format!("{} has no rule targets", record.id)));
    }
    if let Some(registration) = record.registration.as_deref()
        && (!valid_prefixed(registration, "REG-") || !registrations.insert(registration))
    {
        return Err(invalid(format!(
            "{} has malformed or duplicate registration",
            record.id
        )));
    }
    match &record.disposition {
        EvidenceDisposition::Pending { owner_pr, reason } => {
            if record.registration.is_some() {
                return Err(invalid(format!(
                    "{} is pending but has an executable registration",
                    record.id
                )));
            }
            if !valid_pr(owner_pr) || !valid_text(reason) {
                return Err(invalid(format!(
                    "{} has malformed pending disposition",
                    record.id
                )));
            }
        }
        EvidenceDisposition::Complete => {
            if record.registration.is_none() {
                return Err(invalid(format!(
                    "{} is complete without an executable registration",
                    record.id
                )));
            }
        }
    }
    let is_complete = matches!(record.disposition, EvidenceDisposition::Complete);
    let mut local_targets = BTreeSet::new();
    for target in &record.targets {
        if !local_targets.insert((target.rule_id.as_str(), target.requirement_id.as_str())) {
            return Err(invalid(format!("{} has duplicate rule target", record.id)));
        }
        let rule = rule_map.get(target.rule_id.as_str()).ok_or_else(|| {
            invalid(format!(
                "{} references unknown rule {}",
                record.id, target.rule_id
            ))
        })?;
        let requirement = rule
            .requirements
            .iter()
            .find(|item| item.id == target.requirement_id)
            .ok_or_else(|| {
                invalid(format!(
                    "{} references unknown requirement {}",
                    record.id, target.requirement_id
                ))
            })?;
        validate_expectation(&record.id, requirement.kind, record)?;
        let key = (target.rule_id.as_str(), target.requirement_id.as_str());
        if target_states
            .insert(key, is_complete)
            .is_some_and(|state| state != is_complete)
        {
            return Err(invalid(format!(
                "{} mixes complete and pending evidence for one requirement",
                record.id
            )));
        }
    }
    Ok(())
}

fn validate_expectation(
    evidence_id: &str,
    kind: RequirementKind,
    record: &EvidenceRecord,
) -> Result<(), ProfileError> {
    let compatible = match kind {
        RequirementKind::Positive => matches!(&record.expected.status, ExpectedStatus::Success),
        RequirementKind::Negative => matches!(&record.expected.status, ExpectedStatus::Error),
        RequirementKind::ExactStatus => {
            matches!(&record.expected.status, ExpectedStatus::Exact { .. })
        }
        RequirementKind::Inventory => {
            matches!(&record.expected.status, ExpectedStatus::NotApplicable)
        }
        RequirementKind::Model
        | RequirementKind::Differential
        | RequirementKind::Persistence
        | RequirementKind::Crash
        | RequirementKind::Mutation => {
            !matches!(&record.expected.status, ExpectedStatus::NotApplicable)
        }
    };
    if !compatible {
        return Err(invalid(format!(
            "{evidence_id} has an expected status incompatible with its requirement"
        )));
    }
    if let ExpectedStatus::Exact { gqlstatus } = &record.expected.status
        && (gqlstatus.len() != 5
            || !gqlstatus
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase()))
    {
        return Err(invalid(format!("{evidence_id} has malformed GQLSTATUS")));
    }
    if let ExpectedType::Exact { type_name } = &record.expected.result_type
        && !valid_text(type_name)
    {
        return Err(invalid(format!("{evidence_id} has malformed result type")));
    }
    Ok(())
}

fn canonicalize_rules(source: &mut RulesSource) {
    source.approved_domains.sort();
    source.rules.sort_by(|left, right| left.id.cmp(&right.id));
    for rule in &mut source.rules {
        rule.clause_ids.sort();
        rule.features.sort();
        rule.requirements
            .sort_by(|left, right| left.id.cmp(&right.id));
    }
}

fn canonicalize_evidence(source: &mut EvidenceSource) {
    source
        .evidence
        .sort_by(|left, right| left.id.cmp(&right.id));
    for record in &mut source.evidence {
        record.targets.sort();
    }
}

fn target_closure(profile: &ValidatedProfile) -> BTreeSet<String> {
    profile.closure.closure_for(
        profile
            .profile()
            .selected_features
            .iter()
            .map(|item| item.as_str()),
    )
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn unique<'a>(kind: &str, values: impl Iterator<Item = &'a str>) -> Result<(), ProfileError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(invalid(format!("duplicate {kind} {value}")));
        }
    }
    Ok(())
}

fn validate_owner(milestone: &str, owner_pr: &str, owner: &str) -> Result<(), ProfileError> {
    if !valid_milestone(milestone)
        || !valid_pr(owner_pr)
        || !owner_pr.starts_with(&format!("{milestone}-PR"))
    {
        return Err(invalid(format!("{owner} has malformed owner")));
    }
    Ok(())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn valid_prefixed(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|tail| {
        !tail.is_empty()
            && tail.bytes().all(|byte| {
                byte.is_ascii_uppercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
            })
    })
}

fn valid_milestone(value: &str) -> bool {
    matches!(
        value.as_bytes(),
        [b'M', first, second] if first.is_ascii_digit() && second.is_ascii_digit()
    )
}

fn valid_pr(value: &str) -> bool {
    matches!(
        value.as_bytes(),
        [b'M', milestone_a, milestone_b, b'-', b'P', b'R', pr_a, pr_b]
            if milestone_a.is_ascii_digit()
                && milestone_b.is_ascii_digit()
                && pr_a.is_ascii_digit()
                && pr_b.is_ascii_digit()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{
        EvidenceExpectation, EvidenceTarget, ExpectedNullability, ExpectedOrder,
        ExpectedSideEffects,
    };

    #[test]
    fn executable_requirement_categories_decode_and_require_status() {
        for (encoded, kind) in [
            ("\"model\"", RequirementKind::Model),
            ("\"differential\"", RequirementKind::Differential),
            ("\"persistence\"", RequirementKind::Persistence),
            ("\"crash\"", RequirementKind::Crash),
            ("\"mutation\"", RequirementKind::Mutation),
        ] {
            assert_eq!(
                serde_json::from_str::<RequirementKind>(encoded).unwrap(),
                kind
            );
            let mut record = EvidenceRecord {
                id: "EVID-TEST".to_owned(),
                targets: vec![EvidenceTarget {
                    rule_id: "RULE-TEST".to_owned(),
                    requirement_id: "REQ-TEST".to_owned(),
                }],
                expected: EvidenceExpectation {
                    status: ExpectedStatus::NotApplicable,
                    result_type: ExpectedType::NotAsserted,
                    nullability: ExpectedNullability::NotApplicable,
                    ordering: ExpectedOrder::NotApplicable,
                    side_effects: ExpectedSideEffects::NotApplicable,
                },
                registration: None,
                disposition: EvidenceDisposition::Pending {
                    owner_pr: "M01-PR06".to_owned(),
                    reason: "test".to_owned(),
                },
            };
            assert!(validate_expectation(&record.id, kind, &record).is_err());
            record.expected.status = ExpectedStatus::Success;
            assert!(validate_expectation(&record.id, kind, &record).is_ok());
        }
    }
}
