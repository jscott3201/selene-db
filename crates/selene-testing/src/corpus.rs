//! Header-driven parser corpus loader.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use selene_core::feature_register::{FeatureId, feature_id_from_str};

/// Corpus polarity declared by a `.gql` file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorpusKind {
    /// Positive parser example.
    Positive,
    /// Negative parser example.
    Negative,
}

/// Expected parser outcome declared by a `.gql` file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Expectation {
    /// The source must parse and pass the Flagger.
    ParseOk,
    /// The source must reject with the given feature ID.
    ParseRejectedFeature(FeatureId),
    /// The source must reject syntactically or through a builder gap.
    ParseRejectedSyntax,
}

/// One parsed corpus file.
#[derive(Clone, Debug)]
pub struct CorpusCase {
    /// Absolute path to the corpus file.
    pub path: PathBuf,
    /// Declared corpus kind.
    pub kind: CorpusKind,
    /// Primary feature ID.
    pub feature: FeatureId,
    /// Additional feature IDs this case covers.
    pub also_covers: Vec<FeatureId>,
    /// Expected parser outcome.
    pub expectation: Expectation,
    /// Full source text, including header comments.
    pub source: String,
}

impl CorpusCase {
    /// Return all feature IDs declared by this case.
    pub fn declared_features(&self) -> impl Iterator<Item = FeatureId> + '_ {
        std::iter::once(self.feature).chain(self.also_covers.iter().copied())
    }
}

/// Error raised while loading corpus files.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CorpusError {
    /// Filesystem error.
    #[error("{path}: {source}")]
    Io {
        /// Path being read.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: io::Error,
    },
    /// Header was malformed.
    #[error("{path}:{line}: {message}")]
    Header {
        /// Path being parsed.
        path: PathBuf,
        /// One-based line number.
        line: usize,
        /// Human-readable error.
        message: String,
    },
}

/// Load the default tracked corpus directory.
///
/// # Errors
///
/// Returns [`CorpusError`] if any tracked `.gql` file has a malformed header
/// or cannot be read.
pub fn load_default_corpus() -> Result<Vec<CorpusCase>, CorpusError> {
    load_corpus(Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus"))
}

/// Load every `.gql` corpus file under `root`.
///
/// # Errors
///
/// Returns [`CorpusError`] if a file cannot be read or its header is invalid.
pub fn load_corpus(root: impl AsRef<Path>) -> Result<Vec<CorpusCase>, CorpusError> {
    let root = root.as_ref();
    let mut files = Vec::new();
    collect_gql_files(root, &mut files)?;
    files.sort();
    files
        .into_iter()
        .map(|path| load_case(&path))
        .collect::<Result<Vec<_>, _>>()
}

fn collect_gql_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), CorpusError> {
    let entries = fs::read_dir(dir).map_err(|source| CorpusError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CorpusError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_gql_files(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "gql") {
            files.push(path);
        }
    }
    Ok(())
}

fn load_case(path: &Path) -> Result<CorpusCase, CorpusError> {
    let source = fs::read_to_string(path).map_err(|source| CorpusError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_corpus_header(path, source)
}

fn parse_corpus_header(path: &Path, source: String) -> Result<CorpusCase, CorpusError> {
    let mut kind = None;
    let mut feature = None;
    let mut also_covers = Vec::new();
    let mut expectation = None;

    for (index, line) in source.lines().enumerate() {
        let line_no = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("--") else {
            break;
        };
        let directive = rest.trim();
        let Some((key, value)) = directive.split_once(':') else {
            continue;
        };
        match key.trim() {
            "corpus" => kind = Some(parse_kind(path, line_no, value.trim())?),
            "feature" => feature = Some(parse_feature(path, line_no, value.trim())?),
            "also-covers" => {
                also_covers = parse_feature_list(path, line_no, value.trim())?;
            }
            "expects" => expectation = Some(parse_expectation(path, line_no, value.trim())?),
            _ => {}
        }
    }

    let kind = kind.ok_or_else(|| header_error(path, 1, "missing `corpus:` directive"))?;
    let feature = feature.ok_or_else(|| header_error(path, 1, "missing `feature:` directive"))?;
    let expectation =
        expectation.ok_or_else(|| header_error(path, 1, "missing `expects:` directive"))?;

    validate_contract(path, kind, expectation)?;

    Ok(CorpusCase {
        path: path.to_path_buf(),
        kind,
        feature,
        also_covers,
        expectation,
        source,
    })
}

fn parse_kind(path: &Path, line: usize, value: &str) -> Result<CorpusKind, CorpusError> {
    match value {
        "positive" => Ok(CorpusKind::Positive),
        "negative" => Ok(CorpusKind::Negative),
        _ => Err(header_error(
            path,
            line,
            "`corpus:` must be `positive` or `negative`",
        )),
    }
}

fn parse_expectation(path: &Path, line: usize, value: &str) -> Result<Expectation, CorpusError> {
    if value == "parse-ok" {
        return Ok(Expectation::ParseOk);
    }
    let Some(inner) = value
        .strip_prefix("parse-rejected(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Err(header_error(
            path,
            line,
            "`expects:` must be parse-ok or parse-rejected(...)",
        ));
    };
    if inner == "syntax" {
        return Ok(Expectation::ParseRejectedSyntax);
    }
    parse_feature(path, line, inner).map(Expectation::ParseRejectedFeature)
}

fn parse_feature_list(
    path: &Path,
    line: usize,
    value: &str,
) -> Result<Vec<FeatureId>, CorpusError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| parse_feature(path, line, value))
        .collect()
}

fn parse_feature(path: &Path, line: usize, value: &str) -> Result<FeatureId, CorpusError> {
    feature_id_from_str(value).ok_or_else(|| {
        header_error(
            path,
            line,
            format!("unknown ISO feature identifier `{value}`"),
        )
    })
}

fn validate_contract(
    path: &Path,
    kind: CorpusKind,
    expectation: Expectation,
) -> Result<(), CorpusError> {
    match (kind, expectation) {
        (CorpusKind::Positive, Expectation::ParseOk)
        | (CorpusKind::Negative, Expectation::ParseRejectedFeature(_))
        | (CorpusKind::Negative, Expectation::ParseRejectedSyntax) => Ok(()),
        (CorpusKind::Positive, _) => Err(header_error(
            path,
            1,
            "positive corpus files must declare expects: parse-ok",
        )),
        (CorpusKind::Negative, Expectation::ParseOk) => Err(header_error(
            path,
            1,
            "negative corpus files must declare parse rejection",
        )),
    }
}

fn header_error(path: &Path, line: usize, message: impl Into<String>) -> CorpusError {
    CorpusError::Header {
        path: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use selene_core::feature_register::FeatureId;

    fn parse(source: &str) -> CorpusCase {
        parse_corpus_header(Path::new("case.gql"), source.to_owned()).expect("header parses")
    }

    #[test]
    fn parses_positive_header_with_also_covers() {
        let case = parse(
            "-- corpus: positive\n-- feature: GP04\n-- also-covers: GQ03, GQ15\n-- expects: parse-ok\nRETURN 1",
        );
        assert_eq!(case.kind, CorpusKind::Positive);
        assert_eq!(case.feature, FeatureId::GP04);
        assert_eq!(case.also_covers, [FeatureId::GQ03, FeatureId::GQ15]);
        assert_eq!(case.expectation, Expectation::ParseOk);
    }

    #[test]
    fn parses_negative_feature_header() {
        let case = parse(
            "-- corpus: negative\n-- feature: GQ09\n-- expects: parse-rejected(GQ09)\nRETURN 1 OTHERWISE RETURN 2",
        );
        assert_eq!(
            case.expectation,
            Expectation::ParseRejectedFeature(FeatureId::GQ09)
        );
    }

    #[test]
    fn rejects_malformed_contract() {
        let error = parse_corpus_header(
            Path::new("bad.gql"),
            "-- corpus: positive\n-- feature: GQ09\n-- expects: parse-rejected(GQ09)\nRETURN 1"
                .to_owned(),
        )
        .expect_err("contract fails");
        assert!(matches!(error, CorpusError::Header { .. }));
    }
}
