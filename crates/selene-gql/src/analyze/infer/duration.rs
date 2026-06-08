//! Duration value-expression inference for ISO/IEC 39075:2024 §20.28.

use crate::{
    BinaryOp, GqlType, SourceSpan,
    analyze::{
        error::{AnalysisError, ExpectedType, Side, TypeMismatchContext},
        types::AnalyzedType,
    },
};

pub(super) fn duration_add_sub(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Option<Result<AnalyzedType, AnalysisError>> {
    let lhs_is_duration = matches!(lhs, AnalyzedType::Resolved(GqlType::Duration));
    let rhs_is_duration = matches!(rhs, AnalyzedType::Resolved(GqlType::Duration));
    let lhs_is_null = matches!(lhs, AnalyzedType::Resolved(GqlType::Null));
    let rhs_is_null = matches!(rhs, AnalyzedType::Resolved(GqlType::Null));
    let lhs_is_dynamic = matches!(lhs, AnalyzedType::Dynamic);
    let rhs_is_dynamic = matches!(rhs, AnalyzedType::Dynamic);

    match (lhs_is_duration, rhs_is_duration) {
        (true, true) => Some(Ok(AnalyzedType::Resolved(GqlType::Duration))),
        (true, false) if rhs_is_null => Some(Ok(AnalyzedType::Resolved(GqlType::Duration))),
        (false, true) if lhs_is_null => Some(Ok(AnalyzedType::Resolved(GqlType::Duration))),
        (true, false) if rhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
        (false, true) if lhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
        (true, false) => Some(duration_type_mismatch(op, Side::Rhs, rhs, rhs_span)),
        (false, true) => Some(duration_type_mismatch(op, Side::Lhs, lhs, lhs_span)),
        (false, false) => None,
    }
}

fn duration_type_mismatch(
    op: BinaryOp,
    side: Side,
    found: &AnalyzedType,
    span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    let AnalyzedType::Resolved(found) = found else {
        unreachable!("dynamic duration operands are accepted before mismatch construction");
    };
    Err(AnalysisError::TypeMismatch {
        context: TypeMismatchContext::BinaryArithmetic { op, side },
        expected: ExpectedType::Specific(GqlType::Duration),
        found: found.clone(),
        span,
    })
}
