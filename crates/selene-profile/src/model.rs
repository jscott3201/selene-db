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

/// Compatibility status exposed by the current runtime registry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSupport {
    /// Current runtime consumers accept the feature.
    Supported,
    /// Current runtime consumers reject the feature with a rationale.
    Unsupported,
    /// The feature is known but has no parser-reachable rejection surface.
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
    /// Existing runtime compatibility status.
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
    /// Existing runtime compatibility status.
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

/// A selected implementation-defined value.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationDefinedChoiceRecord {
    /// Implementation-defined identifier.
    pub id: ImplDefinedId,
    /// Existing choice text preserved for compatibility.
    pub choice: String,
    /// Existing ownership citation preserved for compatibility.
    pub settled_in: String,
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
    /// Stable profile identifier.
    pub profile_id: String,
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
