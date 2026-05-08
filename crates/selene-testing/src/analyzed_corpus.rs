//! Generic analyzed-corpus harness helpers.
//!
//! This module intentionally does not depend on `selene-gql`; callers pass a
//! parse/analyze closure so `selene-gql` can use the helper from integration
//! tests without creating a Cargo dependency cycle.

use std::{error::Error, fmt, path::PathBuf};

use crate::corpus::{CorpusCase, CorpusError, CorpusKind, Expectation, load_default_corpus};
use selene_gql::{AnalyzedStatement, ProcedureRegistry, analyze, parse};

/// One positive corpus case paired with caller-produced analysis output.
#[derive(Clone, Debug)]
pub struct AnalyzedCorpusCase<T> {
    /// Original corpus metadata.
    pub case: CorpusCase,
    /// Analyzer output produced by the caller.
    pub analyzed: T,
}

/// Error raised while loading or analyzing the positive corpus.
#[derive(Debug)]
pub enum AnalyzedCorpusError<E> {
    /// Corpus loading failed.
    Corpus(CorpusError),
    /// Caller-provided analysis failed for a positive case.
    Analyze {
        /// Path to the case that failed analysis.
        path: PathBuf,
        /// Underlying analyzer error.
        source: E,
    },
}

impl<E> fmt::Display for AnalyzedCorpusError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corpus(source) => write!(f, "{source}"),
            Self::Analyze { path, source } => {
                write!(f, "{}: analysis failed: {source}", path.display())
            }
        }
    }
}

impl<E> Error for AnalyzedCorpusError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Corpus(source) => Some(source),
            Self::Analyze { source, .. } => Some(source),
        }
    }
}

impl<E> From<CorpusError> for AnalyzedCorpusError<E> {
    fn from(source: CorpusError) -> Self {
        Self::Corpus(source)
    }
}

/// Load the default positive corpus and analyze every parse-ok case.
///
/// # Errors
///
/// Returns [`AnalyzedCorpusError`] if the corpus cannot be loaded or if the
/// caller-provided analyzer rejects a positive parse-ok case.
pub fn load_default_analyzed_corpus<T, E>(
    mut analyze_source: impl FnMut(&str) -> Result<T, E>,
) -> Result<Vec<AnalyzedCorpusCase<T>>, AnalyzedCorpusError<E>> {
    load_default_corpus()?
        .into_iter()
        .filter(|case| {
            case.kind == CorpusKind::Positive && case.expectation == Expectation::ParseOk
        })
        .map(|case| {
            let analyzed =
                analyze_source(&case.source).map_err(|source| AnalyzedCorpusError::Analyze {
                    path: case.path.clone(),
                    source,
                })?;
            Ok(AnalyzedCorpusCase { case, analyzed })
        })
        .collect()
}

/// Load the default positive corpus and analyze it with a procedure registry.
///
/// # Errors
///
/// Returns [`AnalyzedCorpusError`] if the corpus cannot be loaded, if parsing a
/// positive case fails, or if semantic analysis rejects a positive case.
pub fn load_default_analyzed_gql_corpus(
    registry: &dyn ProcedureRegistry,
) -> Result<Vec<AnalyzedCorpusCase<AnalyzedStatement>>, AnalyzedCorpusError<String>> {
    load_default_analyzed_corpus(|source| {
        let statement = parse(source).map_err(|err| err.to_string())?;
        analyze(statement, registry).map_err(|err| err.to_string())
    })
}
