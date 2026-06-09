//! Type inference for explicit `TRIM` expressions.

use crate::{
    GqlType, SourceSpan,
    analyze::{
        error::{AnalysisError, ExpectedType, TypeMismatchContext},
        types::AnalyzedType,
    },
};

/// Infer an explicit TRIM expression.
pub(crate) fn trim(
    character: Option<(&AnalyzedType, SourceSpan)>,
    source: &AnalyzedType,
    source_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    let result = match source {
        AnalyzedType::Dynamic => {
            validate_trim_character(character, None)?;
            AnalyzedType::Dynamic
        }
        AnalyzedType::Resolved(GqlType::String) => {
            validate_trim_character(character, Some(GqlType::String))?;
            AnalyzedType::Resolved(GqlType::String)
        }
        AnalyzedType::Resolved(GqlType::Bytes | GqlType::ByteString(_)) => {
            validate_trim_character(character, Some(GqlType::Bytes))?;
            AnalyzedType::Resolved(GqlType::Bytes)
        }
        AnalyzedType::Resolved(GqlType::Null) => match trim_character_result(character)? {
            Some(GqlType::Bytes) => AnalyzedType::Resolved(GqlType::Bytes),
            Some(GqlType::String) | Some(GqlType::Null) | None => {
                AnalyzedType::Resolved(GqlType::String)
            }
            _ => AnalyzedType::Dynamic,
        },
        AnalyzedType::Resolved(actual) => {
            return Err(type_mismatch(
                TypeMismatchContext::TrimSource,
                ExpectedType::StringOrBytes,
                actual.clone(),
                source_span,
            ));
        }
    };
    Ok(result)
}

fn trim_character_result(
    character: Option<(&AnalyzedType, SourceSpan)>,
) -> Result<Option<GqlType>, AnalysisError> {
    let Some((character, span)) = character else {
        return Ok(None);
    };
    match character {
        AnalyzedType::Dynamic => Ok(None),
        AnalyzedType::Resolved(GqlType::String) => Ok(Some(GqlType::String)),
        AnalyzedType::Resolved(GqlType::Null) => Ok(Some(GqlType::Null)),
        AnalyzedType::Resolved(GqlType::Bytes | GqlType::ByteString(_)) => Ok(Some(GqlType::Bytes)),
        AnalyzedType::Resolved(actual) => Err(type_mismatch(
            TypeMismatchContext::TrimCharacter,
            ExpectedType::StringOrBytes,
            actual.clone(),
            span,
        )),
    }
}

fn validate_trim_character(
    character: Option<(&AnalyzedType, SourceSpan)>,
    expected_family: Option<GqlType>,
) -> Result<(), AnalysisError> {
    let Some((actual, span)) = character else {
        return Ok(());
    };
    match (expected_family, actual) {
        (_, AnalyzedType::Dynamic)
        | (_, AnalyzedType::Resolved(GqlType::Null))
        | (Some(GqlType::String), AnalyzedType::Resolved(GqlType::String))
        | (Some(GqlType::Bytes), AnalyzedType::Resolved(GqlType::Bytes | GqlType::ByteString(_))) => {
            Ok(())
        }
        (
            None,
            AnalyzedType::Resolved(GqlType::String | GqlType::Bytes | GqlType::ByteString(_)),
        ) => Ok(()),
        (Some(expected), AnalyzedType::Resolved(actual)) => Err(type_mismatch(
            TypeMismatchContext::TrimCharacter,
            ExpectedType::Specific(expected),
            actual.clone(),
            span,
        )),
        (None, AnalyzedType::Resolved(actual)) => Err(type_mismatch(
            TypeMismatchContext::TrimCharacter,
            ExpectedType::StringOrBytes,
            actual.clone(),
            span,
        )),
    }
}

fn type_mismatch(
    context: TypeMismatchContext,
    expected: ExpectedType,
    found: GqlType,
    span: SourceSpan,
) -> AnalysisError {
    AnalysisError::TypeMismatch {
        context,
        expected,
        found,
        span,
    }
}
