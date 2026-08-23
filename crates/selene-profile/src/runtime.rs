//! Allocation-free compatibility types used by generated data.

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

/// Category-sharded view of the complete Annex B register.
#[derive(Clone, Copy, Debug)]
pub struct AnnexBRegister {
    categories: &'static [&'static [AnnexBRecord]],
}

impl AnnexBRegister {
    pub(crate) const fn new(categories: &'static [&'static [AnnexBRecord]]) -> Self {
        Self { categories }
    }

    /// Return the number of records across all categories.
    #[must_use]
    pub fn len(self) -> usize {
        self.categories.iter().map(|category| category.len()).sum()
    }

    /// Return true when no category contains a record.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.categories.iter().all(|category| category.is_empty())
    }

    /// Iterate records in category and runtime order without allocation.
    pub fn iter(self) -> impl Iterator<Item = &'static AnnexBRecord> {
        self.categories.iter().flat_map(|category| category.iter())
    }

    /// Look up one exact identifier.
    #[must_use]
    pub fn get(self, id: &str) -> Option<&'static AnnexBRecord> {
        self.iter().find(|record| record.id.as_str() == id)
    }
}
