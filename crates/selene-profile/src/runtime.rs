//! Allocation-free runtime types used by generated profile data.

use std::fmt;

/// Stable feature or extension identifier used by runtime consumers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct FeatureId(&'static str);

impl FeatureId {
    pub(crate) const fn new(value: &'static str) -> Self {
        Self(value)
    }

    /// Return the stable identifier text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for FeatureId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Immutable identity of one generated profile contract.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProfileIdentity {
    profile_id: &'static str,
    source_format_version: u32,
    generator_version: u32,
    canonical_hash: &'static str,
}

impl ProfileIdentity {
    /// Construct an identity from static profile coordinates.
    ///
    /// Runtime callers normally use [`crate::current_profile_identity`]. This
    /// constructor permits explicit cache-policy tests with synthetic identities.
    #[must_use]
    pub const fn new(
        profile_id: &'static str,
        source_format_version: u32,
        generator_version: u32,
        canonical_hash: &'static str,
    ) -> Self {
        Self {
            profile_id,
            source_format_version,
            generator_version,
            canonical_hash,
        }
    }

    /// Return the stable target-profile identifier.
    #[must_use]
    pub const fn profile_id(self) -> &'static str {
        self.profile_id
    }

    /// Return the incompatible source-format version.
    #[must_use]
    pub const fn source_format_version(self) -> u32 {
        self.source_format_version
    }

    /// Return the deterministic generator-contract version.
    #[must_use]
    pub const fn generator_version(self) -> u32 {
        self.generator_version
    }

    /// Return the lowercase BLAKE3 hash of the canonical semantic profile.
    #[must_use]
    pub const fn canonical_hash(self) -> &'static str {
        self.canonical_hash
    }
}

/// Fixed session time-zone displacement generated from Annex B ID048.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FixedTimeZoneDisplacement {
    seconds: i32,
}

impl FixedTimeZoneDisplacement {
    pub(crate) const fn new(seconds: i32) -> Self {
        Self { seconds }
    }

    /// Return the signed displacement from UTC in seconds.
    #[must_use]
    pub const fn seconds(self) -> i32 {
        self.seconds
    }
}

/// Generated declared type selected for `SESSION_USER` by Annex B ID061.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SessionUserDeclaredType {
    /// GQL `STRING`.
    String,
}

/// Typed session defaults generated from the selected Annex B records.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionDefaults {
    time_zone: FixedTimeZoneDisplacement,
    initial_parameter_count: u64,
    session_user_declared_type: SessionUserDeclaredType,
}

impl SessionDefaults {
    pub(crate) const fn new(
        time_zone: FixedTimeZoneDisplacement,
        initial_parameter_count: u64,
        session_user_declared_type: SessionUserDeclaredType,
    ) -> Self {
        Self {
            time_zone,
            initial_parameter_count,
            session_user_declared_type,
        }
    }

    /// Return the fixed displacement used by a new session.
    #[must_use]
    pub const fn time_zone(self) -> FixedTimeZoneDisplacement {
        self.time_zone
    }

    /// Return the number of parameters present in a new session.
    #[must_use]
    pub const fn initial_parameter_count(self) -> u64 {
        self.initial_parameter_count
    }

    /// Return the selected declared type for `SESSION_USER`.
    #[must_use]
    pub const fn session_user_declared_type(self) -> SessionUserDeclaredType {
        self.session_user_declared_type
    }
}

/// Complete runtime support state exposed by the feature-status procedure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityStatus {
    /// The complete capability is runtime-supported.
    Supported,
    /// The complete capability is not runtime-supported.
    Unsupported,
    /// The capability is known but has no parser-reachable rejection surface.
    Referenced,
}

impl CapabilityStatus {
    /// Return the stable feature-status spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Referenced => "referenced",
        }
    }
}

/// Parser admission disposition for a generated capability.
///
/// Admission describes the selected parser-visible surface. It is independent
/// of complete runtime support and formal conformance claim state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlaggerStatus {
    /// Parsed use is admitted by the selected profile surface.
    Accepted,
    /// Parsed use is rejected by the Flagger.
    Rejected,
}

impl FlaggerStatus {
    /// Return the stable generated-profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

/// Namespace containing a generated capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureSurface {
    /// ISO optional-feature inventory.
    Iso,
    /// Namespaced implementation extension.
    Extension,
}

impl FeatureSurface {
    /// Return the stable feature-status spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Iso => "iso",
            Self::Extension => "extension",
        }
    }
}

/// Relationship between a capability and the selected target profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileRelation {
    /// Selected explicitly by the target profile.
    Direct,
    /// Included only through Table 10 implication closure.
    Implied,
    /// Known ISO feature outside the target closure.
    Unselected,
    /// Namespaced implementation extension.
    Extension,
}

impl ProfileRelation {
    /// Return the stable feature-status spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Implied => "implied",
            Self::Unselected => "unselected",
            Self::Extension => "extension",
        }
    }
}

/// Formal claim state exposed with a runtime capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityClaimState {
    /// The ISO feature is not implemented for a formal claim.
    Unsupported,
    /// Runtime behavior exists, but no formal claim is made.
    ImplementedUnclaimed,
    /// A claim awaits complete executable evidence.
    ClaimedPendingEvidence,
    /// Complete evidence supports the claim.
    Claimed,
    /// Formal ISO feature claims do not apply to extensions.
    NotApplicable,
}

impl CapabilityClaimState {
    /// Return the stable feature-status spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::ImplementedUnclaimed => "implemented_unclaimed",
            Self::ClaimedPendingEvidence => "claimed_pending_evidence",
            Self::Claimed => "claimed",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// Summary of registered evidence references.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceStatus {
    /// At least one evidence reference is registered.
    Present,
    /// No evidence reference is registered.
    Incomplete,
}

impl EvidenceStatus {
    /// Return the stable feature-status spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Incomplete => "incomplete",
        }
    }
}

/// Complete allocation-free runtime record for one feature or extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRecord {
    /// Stable ISO feature or namespaced extension identifier.
    pub id: FeatureId,
    /// Short display name.
    pub name: &'static str,
    /// Runtime support state.
    pub status: CapabilityStatus,
    /// Parser admission disposition for the selected profile surface.
    pub flagger_status: FlaggerStatus,
    /// ISO or extension namespace.
    pub surface: FeatureSurface,
    /// Direct, implied, unselected, or extension profile relationship.
    pub profile_relation: ProfileRelation,
    /// Formal claim state, or not-applicable for an extension.
    pub claim_state: CapabilityClaimState,
    /// Presence summary for registered evidence references.
    pub evidence_status: EvidenceStatus,
    /// Exact number of registered evidence references.
    pub evidence_count: usize,
    /// Generated non-support rationale; empty for runtime-supported capabilities.
    pub non_support_rationale: &'static str,
}

/// Stable implementation-defined identifier used by runtime consumers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct AnnexBId(&'static str);

impl AnnexBId {
    pub(crate) const fn new(value: &'static str) -> Self {
        Self(value)
    }

    /// Return the implementation-defined identifier text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for AnnexBId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Runtime stability of a selected Annex B value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionStability {
    /// Established profile contract.
    Stable,
    /// Selected value that cannot support a release claim yet.
    Provisional,
}

/// Runtime visibility of a selected Annex B value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionVisibility {
    /// Public GQL-facing contract.
    Public,
    /// Embedder-owned configuration or lifecycle contract.
    Embedder,
    /// Fixed engine behavior without a configuration API.
    Internal,
}

/// Allocation-free typed Annex B value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnexBValue {
    /// Boolean value.
    Boolean(bool),
    /// Non-negative integer value.
    UnsignedInteger(u64),
    /// Identifier-like value.
    Identifier(&'static str),
    /// Text value.
    String(&'static str),
    /// Ordered identifier values.
    OrderedIdentifierList(&'static [&'static str]),
    /// Ordered text values.
    OrderedStringList(&'static [&'static str]),
}

/// Applicability result evaluated from the selected profile closure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicabilityStatus {
    /// The occurrence applies to the selected feature or extension surface.
    Applicable,
    /// The occurrence is absent from that surface.
    NotApplicable,
}

/// Allocation-free Annex B disposition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnexBDecision {
    /// The profile selects a typed value.
    Selected {
        /// Selected value.
        value: AnnexBValue,
        /// Concise implementation-owned explanation.
        rationale: &'static str,
        /// Contract maturity.
        stability: DecisionStability,
        /// Audience that observes or controls the value.
        visibility: DecisionVisibility,
    },
    /// An applicable decision remains owned by a later work item.
    Pending {
        /// Owning 2.0 work item.
        owner: &'static str,
        /// Reason the selection remains unresolved.
        reason: &'static str,
    },
    /// The occurrence does not apply to the selected surface.
    NotApplicable {
        /// Source-backed reason.
        reason: &'static str,
    },
}

/// Complete allocation-free runtime record for one Annex B identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnnexBRecord {
    /// Exact singleton identifier.
    pub id: AnnexBId,
    /// Short implementation-owned topic.
    pub topic: &'static str,
    /// Named applicability expression.
    pub applicability: &'static str,
    /// Evaluated applicability result.
    pub applicability_status: ApplicabilityStatus,
    /// Selected, pending, or not-applicable disposition.
    pub decision: AnnexBDecision,
    /// Short clause citations.
    pub clause_anchors: &'static [&'static str],
    /// Repository evidence identifiers.
    pub evidence: &'static [&'static str],
}
