//! Owned profile source records.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Return the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(FeatureCode, "An ISO optional-feature identifier.");
id_type!(ClauseAnchorId, "A clause-anchor identifier.");
id_type!(ImplicationId, "A feature-implication edge identifier.");
id_type!(
    ImplDefinedId,
    "An implementation-defined choice identifier."
);
id_type!(
    ImplDependentId,
    "An implementation-dependent note identifier."
);
id_type!(ExtensionId, "An implementation-extension identifier.");
id_type!(
    CompatibilityId,
    "A feature or extension identifier used by a compatibility list."
);
id_type!(EvidenceId, "An evidence-reference identifier.");
id_type!(ApplicabilityId, "An applicability-expression identifier.");

/// Formal conformance-claim state, independent of runtime support reporting.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    /// The feature is not implemented for a formal claim.
    Unsupported,
    /// Runtime behavior exists, but no formal claim is made.
    ImplementedUnclaimed,
    /// A claim awaits complete evidence.
    ClaimedPendingEvidence,
    /// Complete evidence supports the claim.
    Claimed,
}

/// Complete runtime support status recorded in the canonical profile inventory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSupport {
    /// The complete capability is runtime-supported.
    Supported,
    /// The complete capability is not runtime-supported.
    Unsupported,
    /// The capability is known but has no parser-reachable rejection surface.
    Referenced,
}

/// A short, non-normative clause citation.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClauseAnchor {
    /// Stable anchor identifier.
    pub id: ClauseAnchorId,
    /// Short citation suitable for generated reports.
    pub citation: String,
}

/// One ISO optional feature in the selected inventory.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRecord {
    /// ISO feature identifier.
    pub id: FeatureCode,
    /// Short feature name.
    pub name: String,
    /// Complete runtime support status.
    pub runtime_support: RuntimeSupport,
    /// Formal claim state.
    pub claim_state: ClaimState,
    /// Exact rejection rationale, or an empty string when none exists.
    pub unsupported_rationale: String,
    /// Stable position in compatibility arrays.
    pub runtime_order: u16,
    /// Applicable clause anchors.
    pub clause_anchors: Vec<ClauseAnchorId>,
    /// Evidence references attached to this record.
    pub evidence: Vec<EvidenceId>,
    /// Applicability expression for this record.
    pub applicability: ApplicabilityId,
}

/// One implementation-defined extension kept distinct from ISO features.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationExtension {
    /// Extension identifier.
    pub id: ExtensionId,
    /// Short extension name.
    pub name: String,
    /// Complete runtime support status.
    pub runtime_support: RuntimeSupport,
    /// Exact rejection rationale, or an empty string when none exists.
    pub unsupported_rationale: String,
    /// Stable position in compatibility arrays.
    pub runtime_order: u16,
    /// Applicable clause anchors.
    pub clause_anchors: Vec<ClauseAnchorId>,
    /// Evidence references attached to this record.
    pub evidence: Vec<EvidenceId>,
    /// Applicability expression for this record.
    pub applicability: ApplicabilityId,
}

/// One direct feature-implication edge.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Implication {
    /// Stable edge identifier.
    pub id: ImplicationId,
    /// Feature that introduces the requirement.
    pub source: FeatureCode,
    /// Required feature.
    pub target: FeatureCode,
    /// Clause anchors supporting the edge.
    pub clause_anchors: Vec<ClauseAnchorId>,
    /// Evidence references supporting the edge.
    pub evidence: Vec<EvidenceId>,
}

/// Stability of a selected implementation-defined value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStability {
    /// The value is an established profile contract.
    Stable,
    /// The value is selected but cannot support a release claim yet.
    Provisional,
}

/// Audience that can observe or configure a selected value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionVisibility {
    /// The value is part of the public GQL-facing contract.
    Public,
    /// The value is selected through the embedder API or product boundary.
    Embedder,
    /// The value describes fixed engine behavior without a configuration API.
    Internal,
}

/// Closed value forms used by selected implementation-defined decisions.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImplementationDefinedValue {
    /// Boolean selection.
    Boolean {
        /// Selected value.
        value: bool,
    },
    /// Non-negative integer selection.
    UnsignedInteger {
        /// Selected value.
        value: u64,
    },
    /// Identifier-like selection.
    Identifier {
        /// Selected identifier.
        value: String,
    },
    /// Text selection that is not an identifier.
    String {
        /// Selected text.
        value: String,
    },
    /// Semantically ordered identifier list.
    OrderedIdentifierList {
        /// Selected identifiers in profile order.
        value: Vec<String>,
    },
    /// Semantically ordered text list.
    OrderedStringList {
        /// Selected values in profile order.
        value: Vec<String>,
    },
}

/// Closed disposition for one implementation-defined occurrence.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImplementationDefinedDecision {
    /// The target profile has selected a value.
    Selected {
        /// Typed selected value.
        value: ImplementationDefinedValue,
        /// Concise implementation-owned explanation.
        rationale: String,
        /// Contract maturity of the selected value.
        stability: DecisionStability,
        /// Audience that observes or controls the value.
        visibility: DecisionVisibility,
    },
    /// The occurrence applies but an owning work item must select its value.
    Pending {
        /// Bounded 2.0 work-item owner.
        owner: String,
        /// Concise reason the value is unresolved.
        reason: String,
    },
    /// The occurrence is absent from the selected feature and extension surface.
    NotApplicable {
        /// Concise source-backed reason.
        reason: String,
    },
}

/// One implementation-defined Annex B record.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationDefinedChoiceRecord {
    /// Implementation-defined identifier.
    pub id: ImplDefinedId,
    /// Short implementation-owned topic label.
    pub topic: String,
    /// Selected, pending, or not-applicable disposition.
    pub decision: ImplementationDefinedDecision,
    /// Stable position in the compatibility array.
    pub runtime_order: u16,
    /// Applicable clause anchors.
    pub clause_anchors: Vec<ClauseAnchorId>,
    /// Evidence references attached to this record.
    pub evidence: Vec<EvidenceId>,
    /// Applicability expression for this record.
    pub applicability: ApplicabilityId,
}

/// An implementation-dependent behavior note.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationDependentNote {
    /// Stable note identifier.
    pub id: ImplDependentId,
    /// Concise behavior note.
    pub note: String,
    /// Applicable clause anchors.
    pub clause_anchors: Vec<ClauseAnchorId>,
    /// Evidence references attached to this record.
    pub evidence: Vec<EvidenceId>,
    /// Applicability expression for this record.
    pub applicability: ApplicabilityId,
}

/// A repository evidence reference.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReference {
    /// Stable evidence identifier.
    pub id: EvidenceId,
    /// Repository-relative reference or source symbol.
    pub reference: String,
    /// Concise description of the evidence.
    pub description: String,
}

/// Validated, non-executable applicability data.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplicabilityExpression {
    /// Always applicable.
    Always,
    /// Applicable when an ISO feature is selected.
    Feature {
        /// Referenced feature.
        feature_id: FeatureCode,
    },
    /// Applicable when an implementation extension is selected.
    Extension {
        /// Referenced extension.
        extension_id: ExtensionId,
    },
    /// Applicable when an implementation-defined choice exists.
    ImplementationDefined {
        /// Referenced choice.
        choice_id: ImplDefinedId,
    },
    /// Reuse another named applicability expression.
    Applicability {
        /// Referenced expression.
        applicability_id: ApplicabilityId,
    },
    /// Every child expression must apply.
    All {
        /// Child expressions.
        items: Vec<ApplicabilityExpression>,
    },
    /// At least one child expression must apply.
    Any {
        /// Child expressions.
        items: Vec<ApplicabilityExpression>,
    },
    /// Negate one expression.
    Not {
        /// Child expression.
        item: Box<ApplicabilityExpression>,
    },
}

/// A named applicability expression.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicabilityDefinition {
    /// Stable expression identifier.
    pub id: ApplicabilityId,
    /// Validated expression tree.
    pub expression: ApplicabilityExpression,
}

/// Complete checked-in profile source format.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Incompatible source-format version.
    pub format_version: u32,
    /// Version of the deterministic generator contract.
    pub generator_version: u32,
    /// Stable profile identifier.
    pub profile_id: String,
    /// ISO features selected directly by this profile.
    pub selected_features: Vec<FeatureCode>,
    /// Whether this profile may be presented as a complete release claim.
    pub release_claimable: bool,
    /// Clause citations used by records.
    pub clause_anchors: Vec<ClauseAnchor>,
    /// ISO optional features.
    pub features: Vec<FeatureRecord>,
    /// Stable order of the legacy supported-feature array.
    pub supported_feature_order: Vec<CompatibilityId>,
    /// Stable order of the legacy non-support rationale array.
    pub unsupported_feature_order: Vec<FeatureCode>,
    /// Direct feature implications.
    pub implications: Vec<Implication>,
    /// Implementation-defined choices.
    pub implementation_defined_choices: Vec<ImplementationDefinedChoiceRecord>,
    /// Implementation-dependent notes.
    pub implementation_dependent_notes: Vec<ImplementationDependentNote>,
    /// Implementation extensions.
    pub implementation_extensions: Vec<ImplementationExtension>,
    /// Evidence references.
    pub evidence: Vec<EvidenceReference>,
    /// Named applicability expressions.
    pub applicability: Vec<ApplicabilityDefinition>,
}
