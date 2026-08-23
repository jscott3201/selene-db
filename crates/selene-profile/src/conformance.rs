//! Static conformance rule and evidence registry records.

mod validate;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ProfileError;

pub use validate::{load_conformance, parse_conformance};

/// Completeness state of the verified rule inventory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryState {
    /// Only reviewed seed records are present.
    SeededIncomplete,
    /// The independently reviewed inventory is complete.
    Complete,
}

/// Target feature boundary governed by the rule source.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeatureScope {
    /// Use the canonical profile's implication-closed target.
    ProfileTargetClosure {
        /// Reviewed closure cardinality.
        expected_count: usize,
        /// BLAKE3 of the sorted feature-ID JSON array.
        feature_ids_hash: String,
    },
}

/// Applicability owned by one static rule record.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuleApplicability {
    /// Applies throughout the seeded boundary.
    Always,
    /// Applies to one selected or implied feature.
    Feature {
        /// Canonical feature identifier.
        feature_id: String,
    },
    /// Applies to the selected profile as a whole.
    Profile,
    /// Applies to the Annex B inventory.
    AnnexB,
}

/// Static evidence category required by a rule.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    /// Successful behavior evidence.
    Positive,
    /// Rejection behavior evidence.
    Negative,
    /// Exact GQLSTATUS evidence.
    ExactStatus,
    /// Reviewed inventory evidence.
    Inventory,
    /// Model-based behavior evidence.
    Model,
    /// Differential behavior evidence.
    Differential,
    /// Persistence and recovery evidence.
    Persistence,
    /// Crash-boundary evidence.
    Crash,
    /// Mutation-testing evidence.
    Mutation,
}

/// One independently addressable rule requirement.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleRequirement {
    /// Rule-local stable requirement identifier.
    pub id: String,
    /// Required evidence category.
    pub kind: RequirementKind,
}

/// One verified project-owned rule identity.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleRecord {
    /// Stable project-owned rule identifier.
    pub id: String,
    /// Concise project-owned label.
    pub label: String,
    /// Approved profile clause anchors.
    pub clause_ids: Vec<String>,
    /// Static applicability.
    pub applicability: RuleApplicability,
    /// Target features addressed by this record.
    pub features: Vec<String>,
    /// Owning 2.0 milestone.
    pub owner_milestone: String,
    /// Owning 2.0 work item.
    pub owner_pr: String,
    /// Required evidence dimensions.
    pub requirements: Vec<RuleRequirement>,
}

/// Closed, independently versioned rule authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesSource {
    /// Incompatible source format version.
    pub format_version: u32,
    /// Rule registry contract version.
    pub registry_version: u32,
    /// Canonical profile identifier.
    pub profile_id: String,
    /// Honest inventory completeness state.
    pub inventory_state: InventoryState,
    /// Exact target feature boundary.
    pub target: FeatureScope,
    /// Closed clause-domain boundary for this seed.
    pub approved_domains: Vec<String>,
    /// Verified rule records.
    pub rules: Vec<RuleRecord>,
}

/// One rule requirement addressed by an evidence record.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceTarget {
    /// Referenced rule identifier.
    pub rule_id: String,
    /// Referenced rule-local requirement identifier.
    pub requirement_id: String,
}

/// Expected status dimension of a planned contract.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectedStatus {
    /// Status does not apply to this static inventory record.
    NotApplicable,
    /// Execution must succeed.
    Success,
    /// Execution must fail without an exact status requirement.
    Error,
    /// Execution must return one exact GQLSTATUS.
    Exact {
        /// Five-character GQLSTATUS code.
        gqlstatus: String,
    },
}

/// Expected result-type dimension of a planned contract.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectedType {
    /// The seed does not yet assert a result type.
    NotAsserted,
    /// The harness must assert this project-owned type label.
    Exact {
        /// Expected type label.
        type_name: String,
    },
}

/// Expected nullability dimension.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedNullability {
    /// Nullability does not apply.
    NotApplicable,
    /// The observed value must be non-null.
    NonNull,
    /// The observed value may be null.
    Nullable,
}

/// Expected ordering dimension.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedOrder {
    /// Ordering does not apply.
    NotApplicable,
    /// No order assertion is planned.
    Unspecified,
    /// The harness must assert exact order.
    Exact,
}

/// Expected side-effect dimension.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedSideEffects {
    /// Side effects do not apply.
    NotApplicable,
    /// The operation must not mutate state.
    Forbidden,
    /// The operation must mutate state as specified by its harness contract.
    Required,
}

/// Closed expected dimensions for one planned evidence contract.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceExpectation {
    /// Status expectation.
    pub status: ExpectedStatus,
    /// Result-type expectation.
    pub result_type: ExpectedType,
    /// Nullability expectation.
    pub nullability: ExpectedNullability,
    /// Ordering expectation.
    pub ordering: ExpectedOrder,
    /// Side-effect expectation.
    pub side_effects: ExpectedSideEffects,
}

/// Current static disposition of an evidence contract.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceDisposition {
    /// The contract remains visible but is not complete.
    Pending {
        /// Work item responsible for the next transition.
        owner_pr: String,
        /// Concise reason the contract remains pending.
        reason: String,
    },
    /// The static contract has completed its owned transition.
    Complete,
}

/// One static evidence contract extending a profile evidence ID.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    /// Existing profile evidence identifier.
    pub id: String,
    /// Rule requirements addressed by this record.
    pub targets: Vec<EvidenceTarget>,
    /// Expected observable dimensions.
    pub expected: EvidenceExpectation,
    /// Planned project-stable compiled registration identifier, when applicable.
    pub planned_registration: Option<String>,
    /// Current static disposition.
    pub disposition: EvidenceDisposition,
}

/// Closed, independently versioned evidence authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSource {
    /// Incompatible source format version.
    pub format_version: u32,
    /// Evidence registry contract version.
    pub registry_version: u32,
    /// Canonical profile identifier.
    pub profile_id: String,
    /// Exact canonical profile hash.
    pub profile_hash: String,
    /// Exact canonical rule registry hash.
    pub rules_hash: String,
    /// Static evidence records.
    pub evidence: Vec<EvidenceRecord>,
}

/// Validated canonical registries and their semantic hashes.
#[derive(Clone, Debug)]
pub struct ValidatedConformance {
    pub(crate) rules: RulesSource,
    pub(crate) evidence: EvidenceSource,
    pub(crate) canonical_rules: Vec<u8>,
    pub(crate) canonical_evidence: Vec<u8>,
    pub(crate) rules_hash: String,
    pub(crate) evidence_hash: String,
}

impl ValidatedConformance {
    /// Return the canonical rule source.
    #[must_use]
    pub fn rules(&self) -> &RulesSource {
        &self.rules
    }

    /// Return the canonical evidence source.
    #[must_use]
    pub fn evidence(&self) -> &EvidenceSource {
        &self.evidence
    }

    /// Return compact canonical rule JSON used as the hash input.
    #[must_use]
    pub fn canonical_rules_json(&self) -> &[u8] {
        &self.canonical_rules
    }

    /// Return compact canonical evidence JSON used as the hash input.
    #[must_use]
    pub fn canonical_evidence_json(&self) -> &[u8] {
        &self.canonical_evidence
    }

    /// Return the canonical BLAKE3 rule hash.
    #[must_use]
    pub fn rules_hash(&self) -> &str {
        &self.rules_hash
    }

    /// Return the canonical BLAKE3 evidence hash.
    #[must_use]
    pub fn evidence_hash(&self) -> &str {
        &self.evidence_hash
    }
}

pub(crate) fn read_source(path: &Path) -> Result<String, ProfileError> {
    std::fs::read_to_string(path).map_err(|source| ProfileError::Io {
        path: PathBuf::from(path),
        source,
    })
}
