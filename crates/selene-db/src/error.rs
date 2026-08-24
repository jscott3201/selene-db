//! Facade-owned diagnostics.

use std::{error::Error as StdError, fmt};

/// Stable category for a facade failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A logical catalog path segment is invalid.
    InvalidCatalogName,
    /// The requested catalog object does not exist.
    CatalogObjectNotFound,
    /// Strict creation found an existing object of the requested kind.
    CatalogObjectAlreadyExists,
    /// A shared-namespace object exists with another kind.
    CatalogObjectWrongKind,
    /// RESTRICT rejected a lifecycle mutation with live dependents or contents.
    CatalogRestrictViolation,
    /// A catalog object reference crosses a prohibited ownership boundary.
    CatalogReferenceViolation,
    /// The temporary bootstrap object cannot be dropped.
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
    /// GQLSTATUS used when a facade mode does not support a GQL feature.
    pub const FEATURE_NOT_SUPPORTED: Self = Self(*b"42N01");
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

impl Error {
    fn facade(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            status: None,
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
            status: None,
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

    /// Return the GQLSTATUS code when the failure came from GQL processing.
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
