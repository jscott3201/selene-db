//! Binary operator expression inference.

use crate::{
    BinaryOp, GqlType, SourceSpan,
    analyze::{
        error::{AnalysisError, ExpectedType, Side, TypeMismatchContext},
        types::AnalyzedType,
    },
};

use super::{
    duration::{duration_add_sub, duration_mul_div, temporal_duration_add_sub},
    ensure_same_comparable_family, expect_boolean, expect_comparable, expect_concat_operand,
    expect_numeric, expect_string, is_byte_string, is_character_string,
    list::list_concat_type,
    meet_gql_types,
    numeric::numeric_promotion,
    type_mismatch,
};

/// Infer a binary operator expression type.
pub(crate) fn binary(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    match op {
        BinaryOp::Add | BinaryOp::Sub => {
            if let Some(result) = temporal_duration_add_sub(op, lhs, lhs_span, rhs, rhs_span) {
                result
            } else if let Some(result) = duration_add_sub(op, lhs, lhs_span, rhs, rhs_span) {
                result
            } else {
                arithmetic(op, lhs, lhs_span, rhs, rhs_span)
            }
        }
        BinaryOp::Mul | BinaryOp::Div => {
            if let Some(result) = duration_mul_div(op, lhs, lhs_span, rhs, rhs_span) {
                result
            } else {
                arithmetic(op, lhs, lhs_span, rhs, rhs_span)
            }
        }
        BinaryOp::Mod => arithmetic(op, lhs, lhs_span, rhs, rhs_span),
        BinaryOp::Power => arithmetic(op, lhs, lhs_span, rhs, rhs_span).map(|ty| match ty {
            AnalyzedType::Dynamic => AnalyzedType::Dynamic,
            AnalyzedType::Resolved(_) => AnalyzedType::Resolved(GqlType::Float),
        }),
        BinaryOp::Eq | BinaryOp::Ne => Ok(AnalyzedType::Resolved(GqlType::Boolean)),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            comparison(op, lhs, lhs_span, rhs, rhs_span)
        }
        BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
            boolean_binary(op, lhs, lhs_span, rhs, rhs_span)
        }
        BinaryOp::Concat => concat(lhs, lhs_span, rhs, rhs_span),
        BinaryOp::Contains | BinaryOp::StartsWith | BinaryOp::EndsWith => {
            string_predicate(op, lhs, lhs_span, rhs, rhs_span)
        }
    }
}

fn arithmetic(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_numeric(
        lhs,
        lhs_span,
        TypeMismatchContext::BinaryArithmetic {
            op,
            side: Side::Lhs,
        },
    )?;
    expect_numeric(
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryArithmetic {
            op,
            side: Side::Rhs,
        },
    )?;
    match (lhs, rhs) {
        (AnalyzedType::Resolved(lhs_ty), AnalyzedType::Resolved(rhs_ty)) => {
            Ok(numeric_promotion(lhs_ty, rhs_ty)
                .map_or(AnalyzedType::Dynamic, AnalyzedType::Resolved))
        }
        (AnalyzedType::Dynamic, _) | (_, AnalyzedType::Dynamic) => Ok(AnalyzedType::Dynamic),
    }
}

fn comparison(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_comparable(
        lhs,
        lhs_span,
        TypeMismatchContext::BinaryComparison {
            op,
            side: Side::Lhs,
        },
    )?;
    expect_comparable(
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryComparison {
            op,
            side: Side::Rhs,
        },
    )?;
    ensure_same_comparable_family(
        lhs,
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryComparison {
            op,
            side: Side::Rhs,
        },
    )?;
    Ok(AnalyzedType::Resolved(GqlType::Boolean))
}

fn boolean_binary(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_boolean(
        lhs,
        lhs_span,
        TypeMismatchContext::BinaryBoolean {
            op,
            side: Side::Lhs,
        },
    )?;
    expect_boolean(
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryBoolean {
            op,
            side: Side::Rhs,
        },
    )?;
    Ok(AnalyzedType::Resolved(GqlType::Boolean))
}

fn concat(
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_concat_operand(
        lhs,
        lhs_span,
        TypeMismatchContext::BinaryConcat { side: Side::Lhs },
    )?;
    expect_concat_operand(
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryConcat { side: Side::Rhs },
    )?;
    match (lhs, rhs) {
        (AnalyzedType::Dynamic, _) | (_, AnalyzedType::Dynamic) => Ok(AnalyzedType::Dynamic),
        (AnalyzedType::Resolved(lhs_ty), AnalyzedType::Resolved(rhs_ty)) => {
            concat_result_type(lhs_ty, rhs_ty)
                .map(AnalyzedType::Resolved)
                .ok_or_else(|| {
                    type_mismatch(
                        TypeMismatchContext::BinaryConcat { side: Side::Rhs },
                        ExpectedType::Specific(lhs_ty.clone()),
                        rhs_ty.clone(),
                        rhs_span,
                    )
                })
        }
    }
}

fn concat_result_type(lhs: &GqlType, rhs: &GqlType) -> Option<GqlType> {
    if matches!(lhs, GqlType::Null) {
        return Some(rhs.strip_not_null().clone());
    }
    if matches!(rhs, GqlType::Null) {
        return Some(lhs.strip_not_null().clone());
    }
    if is_byte_string(lhs) && is_byte_string(rhs) {
        return Some(GqlType::Bytes);
    }
    match (lhs.strip_not_null(), rhs.strip_not_null()) {
        (lhs, rhs) if is_character_string(lhs) && is_character_string(rhs) => Some(GqlType::String),
        (GqlType::Path, GqlType::Path) => Some(GqlType::Path),
        (lhs, rhs) => list_concat_type(lhs, rhs, meet_gql_types),
    }
}

fn string_predicate(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_string(
        lhs,
        lhs_span,
        TypeMismatchContext::BinaryStringPredicate {
            op,
            side: Side::Lhs,
        },
    )?;
    expect_string(
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryStringPredicate {
            op,
            side: Side::Rhs,
        },
    )?;
    Ok(AnalyzedType::Resolved(GqlType::Boolean))
}
