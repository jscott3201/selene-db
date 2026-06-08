//! Duration value-expression inference for ISO/IEC 39075:2024 §20.28.

use crate::{
    BinaryOp, GqlType, SourceSpan,
    analyze::{
        error::{AnalysisError, ExpectedType, Side, TypeMismatchContext},
        types::AnalyzedType,
    },
};

use super::numeric::is_numeric;

pub(super) fn temporal_duration_add_sub(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Option<Result<AnalyzedType, AnalysisError>> {
    if !matches!(op, BinaryOp::Add | BinaryOp::Sub) {
        return None;
    }

    let lhs_temporal = temporal_instant_type(lhs);
    let rhs_temporal = temporal_instant_type(rhs);
    let lhs_is_duration = matches!(lhs, AnalyzedType::Resolved(ty) if ty.is_duration());
    let rhs_is_duration = matches!(rhs, AnalyzedType::Resolved(ty) if ty.is_duration());
    let lhs_is_null = matches!(lhs, AnalyzedType::Resolved(GqlType::Null));
    let rhs_is_null = matches!(rhs, AnalyzedType::Resolved(GqlType::Null));
    let lhs_is_dynamic = matches!(lhs, AnalyzedType::Dynamic);
    let rhs_is_dynamic = matches!(rhs, AnalyzedType::Dynamic);

    match op {
        BinaryOp::Add => match (lhs_temporal, rhs_temporal) {
            (Some(ty), _) if rhs_is_duration || rhs_is_null => {
                Some(Ok(AnalyzedType::Resolved(ty.clone())))
            }
            (Some(_), _) if rhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
            (_, Some(ty)) if lhs_is_duration || lhs_is_null => {
                Some(Ok(AnalyzedType::Resolved(ty.clone())))
            }
            (_, Some(_)) if lhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
            (Some(_), _) => Some(duration_type_mismatch(op, Side::Rhs, rhs, rhs_span)),
            (_, Some(_)) => Some(duration_type_mismatch(op, Side::Lhs, lhs, lhs_span)),
            (None, None) => None,
        },
        BinaryOp::Sub => match lhs_temporal {
            Some(ty) if rhs_is_duration || rhs_is_null => {
                Some(Ok(AnalyzedType::Resolved(ty.clone())))
            }
            Some(_) if rhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
            Some(_) => Some(duration_type_mismatch(op, Side::Rhs, rhs, rhs_span)),
            None => None,
        },
        _ => unreachable!("guarded by temporal_duration_add_sub"),
    }
}

pub(super) fn duration_add_sub(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Option<Result<AnalyzedType, AnalysisError>> {
    let lhs_is_duration = matches!(lhs, AnalyzedType::Resolved(ty) if ty.is_duration());
    let rhs_is_duration = matches!(rhs, AnalyzedType::Resolved(ty) if ty.is_duration());
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

pub(super) fn duration_mul_div(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Option<Result<AnalyzedType, AnalysisError>> {
    if !matches!(op, BinaryOp::Mul | BinaryOp::Div) {
        return None;
    }

    let lhs_is_duration = matches!(lhs, AnalyzedType::Resolved(ty) if ty.is_duration());
    let rhs_is_duration = matches!(rhs, AnalyzedType::Resolved(ty) if ty.is_duration());
    let lhs_is_dynamic = matches!(lhs, AnalyzedType::Dynamic);
    let rhs_is_dynamic = matches!(rhs, AnalyzedType::Dynamic);
    let lhs_is_coefficient = is_coefficient(lhs);
    let rhs_is_coefficient = is_coefficient(rhs);

    match op {
        BinaryOp::Mul => match (lhs_is_duration, rhs_is_duration) {
            (true, false) if rhs_is_coefficient => {
                Some(Ok(AnalyzedType::Resolved(GqlType::Duration)))
            }
            (false, true) if lhs_is_coefficient => {
                Some(Ok(AnalyzedType::Resolved(GqlType::Duration)))
            }
            (true, false) if rhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
            (false, true) if lhs_is_dynamic => Some(Ok(AnalyzedType::Dynamic)),
            (true, false) => Some(numeric_type_mismatch(op, Side::Rhs, rhs, rhs_span)),
            (false, true) => Some(numeric_type_mismatch(op, Side::Lhs, lhs, lhs_span)),
            (false, false) => None,
            (true, true) => Some(numeric_type_mismatch(op, Side::Rhs, rhs, rhs_span)),
        },
        BinaryOp::Div => {
            if lhs_is_duration && rhs_is_coefficient {
                Some(Ok(AnalyzedType::Resolved(GqlType::Duration)))
            } else if lhs_is_duration && rhs_is_dynamic {
                Some(Ok(AnalyzedType::Dynamic))
            } else if lhs_is_duration {
                Some(numeric_type_mismatch(op, Side::Rhs, rhs, rhs_span))
            } else {
                None
            }
        }
        _ => unreachable!("guarded by duration_mul_div"),
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

fn numeric_type_mismatch(
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
        expected: ExpectedType::Numeric,
        found: found.clone(),
        span,
    })
}

fn is_coefficient(ty: &AnalyzedType) -> bool {
    matches!(ty, AnalyzedType::Resolved(GqlType::Null))
        || matches!(ty, AnalyzedType::Resolved(value) if is_numeric(value))
}

fn temporal_instant_type(ty: &AnalyzedType) -> Option<&GqlType> {
    match ty {
        AnalyzedType::Resolved(
            ty @ (GqlType::ZonedDateTime
            | GqlType::LocalDateTime
            | GqlType::Date
            | GqlType::ZonedTime
            | GqlType::LocalTime),
        ) => Some(ty),
        AnalyzedType::Resolved(_) | AnalyzedType::Dynamic => None,
    }
}
