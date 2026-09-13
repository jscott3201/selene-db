//! Closed decode, semantic validation, and canonical hashing.

mod annex_b;
mod applicability;
mod canonical;
pub(crate) mod dependencies;
mod ids;
mod runtime;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::closure::ClosureGraph;
use crate::model::{ApplicabilityExpression, Profile, RuntimeSupport};
use ids::{
    valid_extension_id, valid_feature_id, valid_impl_defined_id, valid_prefixed, valid_profile_id,
};

const FORMAT_VERSION: u32 = 3;
const GENERATOR_VERSION: u32 = 3;
const MAX_APPLICABILITY_DEPTH: usize = 64;

/// Profile loading, validation, or generation failure.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// The profile source could not be read or an output could not be written.
    #[error("profile I/O failed for {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// JSON did not match the closed typed format.
    #[error("profile decode failed: {0}")]
    Decode(#[from] serde_json::Error),
    /// Typed data violated a profile invariant.
    #[error("invalid profile: {0}")]
    Invalid(String),
    /// A checked-in generated file differs from generator output.
    #[error("generated profile output is stale: {0}")]
    Stale(PathBuf),
}

/// A validated, canonically ordered profile and its semantic hash.
#[derive(Clone, Debug)]
pub struct ValidatedProfile {
    profile: Profile,
    canonical_json: Vec<u8>,
    hash: String,
    pub(crate) closure: ClosureGraph,
    pub(crate) applicability: BTreeMap<String, bool>,
}

impl ValidatedProfile {
    /// Return the canonical typed profile.
    #[must_use]
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Return compact canonical JSON used as the hash input.
    #[must_use]
    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    /// Return the lowercase BLAKE3 content hash.
    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Return the evaluated result of a known applicability expression.
    #[must_use]
    pub fn applicability(&self, id: &str) -> Option<bool> {
        self.applicability.get(id).copied()
    }
}

/// Load, decode, validate, and canonicalize a profile file.
pub fn load_profile(path: &Path) -> Result<ValidatedProfile, ProfileError> {
    let source = std::fs::read_to_string(path).map_err(|source| ProfileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_profile(&source)
}

/// Decode, validate, and canonicalize profile JSON.
pub fn parse_profile(source: &str) -> Result<ValidatedProfile, ProfileError> {
    let mut profile: Profile = serde_json::from_str(source)?;
    let (closure, applicability) = validate(&profile)?;
    canonical::canonicalize(&mut profile);
    let canonical_json = serde_json::to_vec(&profile)?;
    let hash = blake3::hash(&canonical_json).to_hex().to_string();
    Ok(ValidatedProfile {
        profile,
        canonical_json,
        hash,
        closure,
        applicability,
    })
}

fn invalid(message: impl Into<String>) -> ProfileError {
    ProfileError::Invalid(message.into())
}

fn validate(profile: &Profile) -> Result<(ClosureGraph, BTreeMap<String, bool>), ProfileError> {
    if profile.format_version != FORMAT_VERSION {
        return Err(invalid(format!(
            "format_version must be {FORMAT_VERSION}, got {}",
            profile.format_version
        )));
    }
    if profile.generator_version != GENERATOR_VERSION {
        return Err(invalid(format!(
            "generator_version must be {GENERATOR_VERSION}, got {}",
            profile.generator_version
        )));
    }
    if !valid_profile_id(&profile.profile_id) {
        return Err(invalid(format!(
            "malformed profile ID {:?}",
            profile.profile_id
        )));
    }

    unique_ids(
        "clause anchor",
        profile.clause_anchors.iter().map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "feature",
        profile.features.iter().map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "selected feature",
        profile.selected_features.iter().map(|item| item.as_str()),
    )?;
    unique_ids(
        "supported compatibility",
        profile
            .supported_feature_order
            .iter()
            .map(|item| item.as_str()),
    )?;
    unique_ids(
        "unsupported compatibility",
        profile
            .unsupported_feature_order
            .iter()
            .map(|item| item.as_str()),
    )?;
    unique_ids(
        "implication",
        profile.implications.iter().map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "implementation-defined choice",
        profile
            .implementation_defined_choices
            .iter()
            .map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "implementation-dependent note",
        profile
            .implementation_dependent_notes
            .iter()
            .map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "implementation extension",
        profile
            .implementation_extensions
            .iter()
            .map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "evidence",
        profile.evidence.iter().map(|item| item.id.as_str()),
    )?;
    unique_ids(
        "applicability",
        profile.applicability.iter().map(|item| item.id.as_str()),
    )?;

    validate_ids(profile)?;
    annex_b::validate_inventory(profile)?;
    runtime::validate(profile)?;
    validate_references(profile)?;
    let closure = ClosureGraph::build(
        profile.features.iter().map(|item| item.id.as_str()),
        profile
            .implications
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str())),
    )
    .map_err(invalid)?;
    validate_applicability_cycles(profile)?;
    let applicability = applicability::evaluate(profile, &closure)?;
    annex_b::validate_decisions(profile, &applicability)?;
    dependencies::validate(profile, &closure)?;
    Ok((closure, applicability))
}

fn unique_ids<'a>(kind: &str, ids: impl Iterator<Item = &'a str>) -> Result<(), ProfileError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(invalid(format!("duplicate {kind} ID {id}")));
        }
    }
    Ok(())
}

fn validate_ids(profile: &Profile) -> Result<(), ProfileError> {
    for item in &profile.clause_anchors {
        require_id("clause anchor", item.id.as_str(), |id| {
            valid_prefixed(id, "CLAUSE-")
        })?;
        require_text("clause citation", &item.citation)?;
    }
    for item in &profile.features {
        require_id("feature", item.id.as_str(), valid_feature_id)?;
        require_text("feature name", &item.name)?;
    }
    for item in &profile.implications {
        require_id("implication", item.id.as_str(), |id| {
            valid_prefixed(id, "IMP-")
        })?;
    }
    for item in &profile.implementation_defined_choices {
        require_id(
            "implementation-defined",
            item.id.as_str(),
            valid_impl_defined_id,
        )?;
        require_text("implementation-defined topic", &item.topic)?;
    }
    for item in &profile.implementation_dependent_notes {
        require_id("implementation-dependent", item.id.as_str(), |id| {
            valid_prefixed(id, "IDN-")
        })?;
        require_text("implementation-dependent note", &item.note)?;
    }
    for item in &profile.implementation_extensions {
        require_id(
            "implementation extension",
            item.id.as_str(),
            valid_extension_id,
        )?;
        require_text("implementation extension name", &item.name)?;
    }
    for item in &profile.evidence {
        require_id("evidence", item.id.as_str(), |id| {
            valid_prefixed(id, "EVID-")
        })?;
        require_text("evidence reference", &item.reference)?;
        require_text("evidence description", &item.description)?;
    }
    for item in &profile.applicability {
        require_id("applicability", item.id.as_str(), |id| {
            valid_prefixed(id, "APP-")
        })?;
    }
    Ok(())
}

fn validate_references(profile: &Profile) -> Result<(), ProfileError> {
    let clauses = strings(profile.clause_anchors.iter().map(|item| item.id.as_str()));
    let features = strings(profile.features.iter().map(|item| item.id.as_str()));
    let extensions = strings(
        profile
            .implementation_extensions
            .iter()
            .map(|item| item.id.as_str()),
    );
    let choices = strings(
        profile
            .implementation_defined_choices
            .iter()
            .map(|item| item.id.as_str()),
    );
    let evidence = strings(profile.evidence.iter().map(|item| item.id.as_str()));
    let applicability = strings(profile.applicability.iter().map(|item| item.id.as_str()));

    check_refs(
        "feature",
        "selected_features",
        &profile.selected_features,
        &features,
    )?;

    let unsupported = profile
        .features
        .iter()
        .filter(|item| item.runtime_support == RuntimeSupport::Unsupported)
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let unsupported_order = profile
        .unsupported_feature_order
        .iter()
        .map(|item| item.as_str())
        .collect::<BTreeSet<_>>();
    if unsupported != unsupported_order {
        return Err(invalid(
            "unsupported_feature_order must contain every runtime-unsupported feature exactly once",
        ));
    }
    for feature in &profile.unsupported_feature_order {
        check_ref(
            "feature",
            "unsupported_feature_order",
            feature.as_str(),
            &features,
        )?;
    }

    for feature in &profile.features {
        check_refs(
            "clause",
            feature.id.as_str(),
            &feature.clause_anchors,
            &clauses,
        )?;
        check_refs(
            "evidence",
            feature.id.as_str(),
            &feature.evidence,
            &evidence,
        )?;
        check_ref(
            "applicability",
            feature.id.as_str(),
            feature.applicability.as_str(),
            &applicability,
        )?;
    }
    for extension in &profile.implementation_extensions {
        check_refs(
            "clause",
            extension.id.as_str(),
            &extension.clause_anchors,
            &clauses,
        )?;
        check_refs(
            "evidence",
            extension.id.as_str(),
            &extension.evidence,
            &evidence,
        )?;
        check_ref(
            "applicability",
            extension.id.as_str(),
            extension.applicability.as_str(),
            &applicability,
        )?;
    }
    for edge in &profile.implications {
        check_ref("feature", edge.id.as_str(), edge.source.as_str(), &features)?;
        check_ref("feature", edge.id.as_str(), edge.target.as_str(), &features)?;
        check_refs("clause", edge.id.as_str(), &edge.clause_anchors, &clauses)?;
        check_refs("evidence", edge.id.as_str(), &edge.evidence, &evidence)?;
    }
    for choice in &profile.implementation_defined_choices {
        check_refs(
            "clause",
            choice.id.as_str(),
            &choice.clause_anchors,
            &clauses,
        )?;
        check_refs("evidence", choice.id.as_str(), &choice.evidence, &evidence)?;
        check_ref(
            "applicability",
            choice.id.as_str(),
            choice.applicability.as_str(),
            &applicability,
        )?;
    }
    for note in &profile.implementation_dependent_notes {
        check_refs("clause", note.id.as_str(), &note.clause_anchors, &clauses)?;
        check_refs("evidence", note.id.as_str(), &note.evidence, &evidence)?;
        check_ref(
            "applicability",
            note.id.as_str(),
            note.applicability.as_str(),
            &applicability,
        )?;
    }
    for definition in &profile.applicability {
        validate_expression(
            &definition.expression,
            definition.id.as_str(),
            0,
            &features,
            &extensions,
            &choices,
            &applicability,
        )?;
    }
    Ok(())
}

fn validate_expression(
    expression: &ApplicabilityExpression,
    owner: &str,
    depth: usize,
    features: &BTreeSet<&str>,
    extensions: &BTreeSet<&str>,
    choices: &BTreeSet<&str>,
    applicability: &BTreeSet<&str>,
) -> Result<(), ProfileError> {
    if depth > MAX_APPLICABILITY_DEPTH {
        return Err(invalid(format!(
            "{owner} exceeds applicability depth {MAX_APPLICABILITY_DEPTH}"
        )));
    }
    match expression {
        ApplicabilityExpression::Always => Ok(()),
        ApplicabilityExpression::Feature { feature_id } => {
            check_ref("feature", owner, feature_id.as_str(), features)
        }
        ApplicabilityExpression::Extension { extension_id } => {
            check_ref("extension", owner, extension_id.as_str(), extensions)
        }
        ApplicabilityExpression::ImplementationDefined { choice_id } => {
            check_ref("implementation-defined", owner, choice_id.as_str(), choices)
        }
        ApplicabilityExpression::Applicability { applicability_id } => check_ref(
            "applicability",
            owner,
            applicability_id.as_str(),
            applicability,
        ),
        ApplicabilityExpression::All { items } | ApplicabilityExpression::Any { items } => {
            if items.is_empty() {
                return Err(invalid(format!("{owner} has an empty applicability group")));
            }
            for item in items {
                validate_expression(
                    item,
                    owner,
                    depth + 1,
                    features,
                    extensions,
                    choices,
                    applicability,
                )?;
            }
            Ok(())
        }
        ApplicabilityExpression::Not { item } => validate_expression(
            item,
            owner,
            depth + 1,
            features,
            extensions,
            choices,
            applicability,
        ),
    }
}

fn validate_applicability_cycles(profile: &Profile) -> Result<(), ProfileError> {
    let mut applicability = BTreeMap::<&str, Vec<&str>>::new();
    for definition in &profile.applicability {
        let mut references = Vec::new();
        collect_applicability_refs(&definition.expression, &mut references);
        applicability.insert(definition.id.as_str(), references);
    }
    check_cycles("applicability", &applicability)
}

fn collect_applicability_refs<'a>(
    expression: &'a ApplicabilityExpression,
    output: &mut Vec<&'a str>,
) {
    match expression {
        ApplicabilityExpression::Applicability { applicability_id } => {
            output.push(applicability_id.as_str());
        }
        ApplicabilityExpression::All { items } | ApplicabilityExpression::Any { items } => {
            for item in items {
                collect_applicability_refs(item, output);
            }
        }
        ApplicabilityExpression::Not { item } => collect_applicability_refs(item, output),
        ApplicabilityExpression::Always
        | ApplicabilityExpression::Feature { .. }
        | ApplicabilityExpression::Extension { .. }
        | ApplicabilityExpression::ImplementationDefined { .. } => {}
    }
}

fn check_cycles(kind: &str, graph: &BTreeMap<&str, Vec<&str>>) -> Result<(), ProfileError> {
    fn visit<'a>(
        node: &'a str,
        kind: &str,
        graph: &BTreeMap<&'a str, Vec<&'a str>>,
        state: &mut BTreeMap<&'a str, u8>,
        stack: &mut Vec<&'a str>,
    ) -> Result<(), ProfileError> {
        state.insert(node, 1);
        stack.push(node);
        let mut targets = graph[node].clone();
        targets.sort_unstable();
        for target in targets {
            if state[target] == 1 {
                let start = stack.iter().position(|item| *item == target).unwrap_or(0);
                let mut cycle = stack[start..].to_vec();
                cycle.push(target);
                return Err(invalid(format!("{kind} cycle: {}", cycle.join(" -> "))));
            }
            if state[target] == 0 {
                visit(target, kind, graph, state, stack)?;
            }
        }
        stack.pop();
        state.insert(node, 2);
        Ok(())
    }

    let mut state = graph
        .keys()
        .map(|node| (*node, 0))
        .collect::<BTreeMap<_, _>>();
    let mut stack = Vec::new();
    for node in graph.keys().copied() {
        if state[node] == 0 {
            visit(node, kind, graph, &mut state, &mut stack)?;
        }
    }
    Ok(())
}

fn strings<'a>(values: impl Iterator<Item = &'a str>) -> BTreeSet<&'a str> {
    values.collect()
}

fn check_refs<T: AsRefId>(
    kind: &str,
    owner: &str,
    refs: &[T],
    known: &BTreeSet<&str>,
) -> Result<(), ProfileError> {
    let mut seen = BTreeSet::new();
    for reference in refs {
        let id = reference.as_ref_id();
        if !seen.insert(id) {
            return Err(invalid(format!(
                "{owner} has duplicate {kind} reference {id}"
            )));
        }
        check_ref(kind, owner, id, known)?;
    }
    Ok(())
}

trait AsRefId {
    fn as_ref_id(&self) -> &str;
}

macro_rules! ref_id {
    ($($name:ty),+ $(,)?) => {$(
        impl AsRefId for $name {
            fn as_ref_id(&self) -> &str {
                self.as_str()
            }
        }
    )+};
}

ref_id!(crate::ClauseAnchorId, crate::EvidenceId, crate::FeatureCode);

fn check_ref(
    kind: &str,
    owner: &str,
    reference: &str,
    known: &BTreeSet<&str>,
) -> Result<(), ProfileError> {
    if known.contains(reference) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{owner} references unknown {kind} ID {reference}"
        )))
    }
}

fn require_id(kind: &str, id: &str, predicate: fn(&str) -> bool) -> Result<(), ProfileError> {
    if predicate(id) {
        Ok(())
    } else {
        Err(invalid(format!("malformed {kind} ID {id}")))
    }
}

fn require_text(kind: &str, text: &str) -> Result<(), ProfileError> {
    if text.is_empty() || text.trim() != text || text.chars().any(char::is_control) {
        Err(invalid(format!("{kind} must be non-empty, trimmed text")))
    } else {
        Ok(())
    }
}
