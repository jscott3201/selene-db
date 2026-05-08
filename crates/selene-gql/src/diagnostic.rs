//! Source-carrying parser diagnostic wrapper.

use std::sync::Arc;

use miette::NamedSource;

use crate::ParserError;

/// Parser error plus the source text needed for terminal caret rendering.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{inner}")]
#[diagnostic(forward(inner))]
pub struct DiagnosticReport {
    #[source_code]
    source_code: Arc<NamedSource<Arc<str>>>,
    inner: ParserError,
}

impl DiagnosticReport {
    /// Construct a source-carrying diagnostic.
    #[must_use]
    pub fn new(error: ParserError, source: Arc<str>, label: impl Into<String>) -> Self {
        Self {
            source_code: Arc::new(NamedSource::new(label.into(), source)),
            inner: error,
        }
    }

    /// Return the wrapped parser error.
    #[must_use]
    pub const fn error(&self) -> &ParserError {
        &self.inner
    }

    /// Return the source text attached to this diagnostic.
    #[must_use]
    pub fn source(&self) -> &str {
        self.source_code.inner().as_ref()
    }

    /// Return the named source attached to this diagnostic.
    #[must_use]
    pub fn named_source(&self) -> &NamedSource<Arc<str>> {
        &self.source_code
    }
}
