//! Facade-owned diagnostics.

use std::{error::Error as StdError, fmt};

/// Stable category for a facade failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
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
