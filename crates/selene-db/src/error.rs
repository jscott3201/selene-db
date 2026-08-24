//! Facade-owned diagnostics.

use std::{error::Error as StdError, fmt};

/// Stable category for a facade failure.
///
/// Catalog categories carry a fixed GQLSTATUS (see [`ErrorKind::gqlstatus`])
/// so Rust and GQL callers observe the same diagnostic for the same failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A logical catalog path segment is invalid (`42001`: the name fails the
    /// selected identifier profile).
    InvalidCatalogName,
    /// The requested catalog object, or a parent it needs, does not exist, or
    /// the reference has a shape the catalog cannot hold (`42002`, ISO/IEC
    /// 39075:2024 Table 8 "invalid reference"; §23.2 GR2d).
    CatalogObjectNotFound,
    /// Strict creation found an existing object of the requested kind
    /// (`42N10`, a selene-db subclass; ISO delegates the condition to the
    /// implementation under IE005).
    CatalogObjectAlreadyExists,
    /// A shared-namespace object exists with another kind (`42002`: §17.2
    /// SR2d(i)(1) makes the wrong kind a reference syntax-rule violation).
    CatalogObjectWrongKind,
    /// RESTRICT rejected a lifecycle mutation with live dependents or contents
    /// (`G1000`, the class-level "dependent object error"; not `G1001`, whose
    /// subclass names edges only).
    CatalogRestrictViolation,
    /// A catalog object reference crosses a prohibited ownership boundary
    /// (`42002`).
    CatalogReferenceViolation,
    /// The temporary bootstrap object cannot be dropped (`42000`, the
    /// class-level "syntax error or access rule violation"; the access rule is
    /// implementation-defined under IE005).
    ProtectedCatalogObject,
    /// A graph handle's stable identity is no longer registered.
    StaleGraphHandle,
    /// A facade graph-type definition is inconsistent.
    InvalidGraphType,
    /// Catalog identity or immutable-state validation failed internally.
    CatalogInvariant,
    /// The source is not valid GQL.
    InvalidGql,
    /// The requested GQL feature is not supported by this facade mode.
    FeatureNotSupported,
    /// Parsing, analysis, planning, or execution failed for another reason.
    Execution,
}

/// Facade-owned five-character GQLSTATUS code.
///
/// This value copies the semantic code emitted by the GQL engine without
/// re-exporting the lower crate's status type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GqlStatus([u8; 5]);

impl GqlStatus {
    /// Completion condition of a successful catalog statement with an omitted
    /// result (ISO/IEC 39075:2024 §4.9.3).
    pub const SUCCESSFUL_COMPLETION_OMITTED_RESULT: Self = Self(*b"00001");
    /// Completion condition of `DROP GRAPH IF EXISTS` on an absent graph
    /// (§12.5 GR1).
    pub const GRAPH_DOES_NOT_EXIST: Self = Self(*b"01G03");
    /// Class-level "syntax error or access rule violation", used for
    /// protected bootstrap objects.
    pub const SYNTAX_ERROR_OR_ACCESS_RULE_VIOLATION: Self = Self(*b"42000");
    /// "Invalid syntax", used when a catalog name fails the identifier profile.
    pub const SYNTAX_ERROR: Self = Self(*b"42001");
    /// "Invalid reference", used for missing, wrong-kind, and unshapeable
    /// catalog references.
    pub const INVALID_REFERENCE: Self = Self(*b"42002");
    /// GQLSTATUS used when a facade mode does not support a GQL feature.
    pub const FEATURE_NOT_SUPPORTED: Self = Self(*b"42N01");
    /// Duplicate catalog object under strict creation (selene-db subclass).
    pub const DUPLICATE_OBJECT: Self = Self(*b"42N10");
    /// Class-level "dependent object error", used for RESTRICT violations.
    pub const DEPENDENT_OBJECT_ERROR: Self = Self(*b"G1000");
    /// GQLSTATUS used when a closed graph violates its bound graph type.
    pub const GRAPH_TYPE_VIOLATION: Self = Self(*b"G2000");

    pub(crate) fn from_engine(status: selene_gql::GqlStatus) -> Self {
        let mut code = [0_u8; 5];
        code.copy_from_slice(status.as_str().as_bytes());
        Self(code)
    }

    /// Return the five-character code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("5GQL0")
    }
}

impl fmt::Display for GqlStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Failure returned by a facade operation.
///
/// Public accessors return owned facade data. [`StdError::source`] retains the
/// concrete engine diagnostic behind a trait object for logs and error chains;
/// no lower error type appears in the facade signature.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    status: Option<GqlStatus>,
    message: String,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl ErrorKind {
    /// Return the GQLSTATUS the facade assigns to this category, if any.
    ///
    /// Engine-originated categories take their status from the engine
    /// diagnostic instead; internal invariant, stale-handle, and graph-type
    /// definition failures have none.
    #[must_use]
    pub const fn gqlstatus(self) -> Option<GqlStatus> {
        match self {
            Self::InvalidCatalogName => Some(GqlStatus::SYNTAX_ERROR),
            Self::CatalogObjectNotFound
            | Self::CatalogObjectWrongKind
            | Self::CatalogReferenceViolation => Some(GqlStatus::INVALID_REFERENCE),
            Self::CatalogObjectAlreadyExists => Some(GqlStatus::DUPLICATE_OBJECT),
            Self::CatalogRestrictViolation => Some(GqlStatus::DEPENDENT_OBJECT_ERROR),
            Self::ProtectedCatalogObject => Some(GqlStatus::SYNTAX_ERROR_OR_ACCESS_RULE_VIOLATION),
            Self::StaleGraphHandle
            | Self::InvalidGraphType
            | Self::CatalogInvariant
            | Self::InvalidGql
            | Self::FeatureNotSupported
            | Self::Execution => None,
        }
    }
}

impl Error {
    fn facade(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            status: kind.gqlstatus(),
            message: message.into(),
            source: None,
        }
    }

    fn with_source(
        kind: ErrorKind,
        message: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            status: kind.gqlstatus(),
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub(crate) fn from_engine(source: selene_gql::ExecutorError) -> Self {
        let status = GqlStatus::from_engine(source.gqlstatus());
        let kind = match status.as_str() {
            "42001" => ErrorKind::InvalidGql,
            "42N01" => ErrorKind::FeatureNotSupported,
            _ => ErrorKind::Execution,
        };
        Self {
            kind,
            status: Some(status),
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    pub(crate) fn unsupported_engine_outcome() -> Self {
        Self {
            kind: ErrorKind::Execution,
            status: None,
            message: "the GQL engine returned an outcome unknown to this facade".to_owned(),
            source: None,
        }
    }

    pub(crate) fn from_catalog_name(source: selene_catalog::CatalogError) -> Self {
        Self::with_source(ErrorKind::InvalidCatalogName, source.to_string(), source)
    }

    pub(crate) fn from_catalog_invariant(source: selene_catalog::CatalogError) -> Self {
        Self::with_source(ErrorKind::CatalogInvariant, source.to_string(), source)
    }

    pub(crate) fn invalid_graph_type_source(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::with_source(ErrorKind::InvalidGraphType, source.to_string(), source)
    }

    pub(crate) fn invalid_graph_type(message: impl Into<String>) -> Self {
        Self::facade(ErrorKind::InvalidGraphType, message)
    }

    pub(crate) fn not_found(path: &impl fmt::Display, kind: &str) -> Self {
        Self::facade(
            ErrorKind::CatalogObjectNotFound,
            format!("{kind} {path} does not exist"),
        )
    }

    /// A GQL reference that no facade path can represent.
    pub(crate) fn invalid_reference(reference: &str, detail: &str) -> Self {
        Self::facade(
            ErrorKind::CatalogObjectNotFound,
            format!("invalid catalog reference {reference}: {detail}"),
        )
    }

    pub(crate) fn already_exists(path: &impl fmt::Display) -> Self {
        Self::facade(
            ErrorKind::CatalogObjectAlreadyExists,
            format!("catalog object {path} already exists"),
        )
    }

    pub(crate) fn wrong_kind(
        path: &impl fmt::Display,
        expected: &str,
        actual: selene_catalog::CatalogObjectKind,
    ) -> Self {
        Self::facade(
            ErrorKind::CatalogObjectWrongKind,
            format!("catalog path {path} is a {actual}, not a {expected}"),
        )
    }

    pub(crate) fn protected(path: &impl fmt::Display, kind: &str) -> Self {
        Self::facade(
            ErrorKind::ProtectedCatalogObject,
            format!("{kind} {path} is protected until M02-PR05"),
        )
    }

    pub(crate) fn stale_graph(path: &impl fmt::Display) -> Self {
        Self::facade(
            ErrorKind::StaleGraphHandle,
            format!("graph handle for {path} is stale or invalidated"),
        )
    }

    pub(crate) fn nonempty_graph(path: &impl fmt::Display, nodes: usize, edges: usize) -> Self {
        Self::facade(
            ErrorKind::CatalogRestrictViolation,
            format!(
                "cannot drop nonempty graph {path} under RESTRICT: {nodes} nodes, {edges} edges"
            ),
        )
    }

    pub(crate) fn nonempty_schema(path: &impl fmt::Display, children: usize) -> Self {
        Self::facade(
            ErrorKind::CatalogRestrictViolation,
            format!("cannot drop nonempty schema {path} under RESTRICT: {children} objects"),
        )
    }

    pub(crate) fn referenced_graph_type(path: &impl fmt::Display, references: usize) -> Self {
        Self::facade(
            ErrorKind::CatalogRestrictViolation,
            format!(
                "cannot drop graph type {path} under RESTRICT: referenced by {references} graphs"
            ),
        )
    }

    pub(crate) fn cross_schema_graph_type(path: &impl fmt::Display) -> Self {
        Self::facade(
            ErrorKind::CatalogReferenceViolation,
            format!("graph type {path} is not in the graph's schema"),
        )
    }

    pub(crate) fn catalog_invariant(message: impl Into<String>) -> Self {
        Self::facade(ErrorKind::CatalogInvariant, message)
    }

    pub(crate) fn identifier_exhausted(kind: &str) -> Self {
        Self::facade(
            ErrorKind::CatalogInvariant,
            format!("{kind} ID high-water mark is exhausted"),
        )
    }

    #[cfg(test)]
    pub(crate) fn injected_failure(stage: &str) -> Self {
        Self::facade(
            ErrorKind::CatalogInvariant,
            format!("test failure injected {stage}"),
        )
    }

    /// Return the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Return the GQLSTATUS code of this failure.
    ///
    /// Catalog failures carry the code selected by their [`ErrorKind`] whether
    /// the request came from Rust or GQL; engine failures copy the engine's
    /// code; internal invariant, stale-handle, and graph-type definition
    /// failures have none.
    #[must_use]
    pub const fn gqlstatus(&self) -> Option<GqlStatus> {
        self.status
    }

    /// Return the owned diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}
