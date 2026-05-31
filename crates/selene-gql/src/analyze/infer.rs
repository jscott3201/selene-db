//! Expression type-inference helpers.

mod numeric;

use crate::{
    BinaryOp, GqlType, IsCheckKind, Literal, RecordType, SourceSpan, UnaryOp,
    analyze::{
        error::{AnalysisError, ConditionClause, ExpectedType, Side, TypeMismatchContext},
        types::AnalyzedType,
    },
};

use self::numeric::{is_numeric, numeric_promotion};

pub(crate) use self::numeric::argument_assignable;

/// Infer a literal expression type.
#[must_use]
pub(crate) fn literal(literal: &Literal) -> AnalyzedType {
    match literal {
        Literal::Bool(..) => AnalyzedType::Resolved(GqlType::Boolean),
        Literal::Integer(..) => AnalyzedType::Resolved(GqlType::Integer),
        Literal::Float(..) => AnalyzedType::Resolved(GqlType::Float),
        Literal::String(..) => AnalyzedType::Resolved(GqlType::String),
        Literal::Uuid(..) => AnalyzedType::Resolved(GqlType::Uuid),
        Literal::Null(..) => AnalyzedType::Resolved(GqlType::Null),
    }
}

/// Infer a binary operator expression type.
pub(crate) fn binary(
    op: BinaryOp,
    lhs: &AnalyzedType,
    lhs_span: SourceSpan,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    match op {
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Mod
        | BinaryOp::Power => arithmetic(op, lhs, lhs_span, rhs, rhs_span),
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

/// Infer a unary operator expression type.
pub(crate) fn unary(
    op: UnaryOp,
    operand: &AnalyzedType,
    span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    match op {
        UnaryOp::Negate => match operand {
            AnalyzedType::Dynamic => Ok(AnalyzedType::Dynamic),
            AnalyzedType::Resolved(ty) if is_numeric(ty) => Ok(operand.clone()),
            // Per ISO/IEC 39075:2024 three-valued logic, `- NULL` yields NULL.
            // The runtime returns `Value::Null`; the analyzer must not reject.
            AnalyzedType::Resolved(GqlType::Null) => Ok(AnalyzedType::Resolved(GqlType::Null)),
            AnalyzedType::Resolved(found) => Err(type_mismatch(
                TypeMismatchContext::UnaryNegate,
                ExpectedType::Numeric,
                found.clone(),
                span,
            )),
        },
        UnaryOp::Not => match operand {
            AnalyzedType::Dynamic | AnalyzedType::Resolved(GqlType::Boolean) => {
                Ok(AnalyzedType::Resolved(GqlType::Boolean))
            }
            // `NOT NULL` yields NULL (UNKNOWN) under three-valued logic; the
            // runtime returns `Value::Null`. The static result type stays
            // Boolean (the UNKNOWN truth value lives in the Boolean domain).
            AnalyzedType::Resolved(GqlType::Null) => Ok(AnalyzedType::Resolved(GqlType::Boolean)),
            AnalyzedType::Resolved(found) => Err(type_mismatch(
                TypeMismatchContext::UnaryNot,
                ExpectedType::Boolean,
                found.clone(),
                span,
            )),
        },
    }
}

/// Infer an `IS` predicate type and validate statically-checkable operands.
pub(crate) fn is_check(
    kind: &IsCheckKind,
    operand: &AnalyzedType,
    operand_span: SourceSpan,
    predicate_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    match kind {
        IsCheckKind::Typed(ty) if !is_supported_typed_target(ty) => Err(type_mismatch(
            TypeMismatchContext::IsTypedTarget,
            ExpectedType::Comparable,
            ty.clone(),
            predicate_span,
        )),
        IsCheckKind::Normalized(_) => {
            expect_string(operand, operand_span, TypeMismatchContext::IsNormalized)?;
            Ok(AnalyzedType::Resolved(GqlType::Boolean))
        }
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::SourceOf(_)
        | IsCheckKind::DestinationOf(_) => Ok(AnalyzedType::Resolved(GqlType::Boolean)),
    }
}

/// Infer `NORMALIZE(<string>[, <normal form>])`.
pub(crate) fn normalize(
    source: &AnalyzedType,
    source_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_string(source, source_span, TypeMismatchContext::NormalizeFunction)?;
    Ok(AnalyzedType::Resolved(GqlType::String))
}

/// Infer an explicit TRIM source expression.
pub(crate) fn trim_source(
    source: &AnalyzedType,
    source_span: SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    expect_string(source, source_span, TypeMismatchContext::TrimSource)?;
    Ok(AnalyzedType::Resolved(GqlType::String))
}

/// Infer an `IN` predicate type.
pub(crate) fn in_list(
    operand: &AnalyzedType,
    operand_span: SourceSpan,
    items: &[(AnalyzedType, SourceSpan)],
) -> Result<AnalyzedType, AnalysisError> {
    let mut unified: Option<(GqlType, SourceSpan)> = None;
    for (ty, span) in items {
        if let AnalyzedType::Resolved(item_ty) = ty {
            if let Some((current, _)) = &unified {
                if let Some(meet) = meet_gql_types(current, item_ty) {
                    unified = Some((meet, *span));
                } else {
                    return Err(type_mismatch(
                        TypeMismatchContext::InListUnification,
                        ExpectedType::Specific(current.clone()),
                        item_ty.clone(),
                        *span,
                    ));
                }
            } else {
                unified = Some((item_ty.clone(), *span));
            }
        }
    }
    if let (AnalyzedType::Resolved(operand_ty), Some((item_ty, item_span))) = (operand, unified)
        && meet_gql_types(operand_ty, &item_ty).is_none()
    {
        return Err(type_mismatch(
            TypeMismatchContext::InListUnification,
            ExpectedType::Specific(operand_ty.clone()),
            item_ty,
            item_span.max(operand_span),
        ));
    }
    Ok(AnalyzedType::Resolved(GqlType::Boolean))
}

/// Infer a list literal type.
pub(crate) fn list_literal(
    items: &[(AnalyzedType, SourceSpan)],
) -> Result<AnalyzedType, AnalysisError> {
    let mut unified: Option<(GqlType, SourceSpan)> = None;
    let mut saw_dynamic = false;
    for (ty, span) in items {
        match ty {
            AnalyzedType::Dynamic => saw_dynamic = true,
            AnalyzedType::Resolved(item_ty) => {
                unified = Some(match unified {
                    Some((ref current, _)) => (
                        meet_gql_types(current, item_ty).ok_or_else(|| {
                            type_mismatch(
                                TypeMismatchContext::ListLiteralUnification,
                                ExpectedType::Specific(current.clone()),
                                item_ty.clone(),
                                *span,
                            )
                        })?,
                        *span,
                    ),
                    None => (item_ty.clone(), *span),
                });
            }
        }
    }
    if saw_dynamic {
        return Ok(AnalyzedType::Dynamic);
    }
    let Some((item_ty, _)) = unified else {
        return Ok(AnalyzedType::Dynamic);
    };
    Ok(AnalyzedType::Resolved(GqlType::List(Box::new(item_ty))))
}

/// Infer a CASE expression result type from branch result cells.
pub(crate) fn case_result(
    branches: &[(AnalyzedType, SourceSpan)],
) -> Result<AnalyzedType, AnalysisError> {
    let mut unified: Option<(GqlType, SourceSpan)> = None;
    let mut saw_dynamic = false;
    for (ty, span) in branches {
        match ty {
            AnalyzedType::Dynamic => saw_dynamic = true,
            AnalyzedType::Resolved(branch_ty) => {
                unified = Some(match unified {
                    Some((ref current, _)) => (
                        meet_gql_types(current, branch_ty).ok_or_else(|| {
                            type_mismatch(
                                TypeMismatchContext::CaseBranchUnification,
                                ExpectedType::Specific(current.clone()),
                                branch_ty.clone(),
                                *span,
                            )
                        })?,
                        *span,
                    ),
                    None => (branch_ty.clone(), *span),
                });
            }
        }
    }
    if saw_dynamic {
        return Ok(AnalyzedType::Dynamic);
    }
    Ok(unified
        .map(|(ty, _)| AnalyzedType::Resolved(ty))
        .unwrap_or(AnalyzedType::Dynamic))
}

/// Ensure a clause condition is boolean when statically known.
pub(crate) fn condition(
    ty: &AnalyzedType,
    span: SourceSpan,
    clause: ConditionClause,
) -> Result<(), AnalysisError> {
    match ty {
        AnalyzedType::Dynamic | AnalyzedType::Resolved(GqlType::Boolean) => Ok(()),
        AnalyzedType::Resolved(found) => Err(type_mismatch(
            TypeMismatchContext::Condition { clause },
            ExpectedType::Specific(GqlType::Boolean),
            found.clone(),
            span,
        )),
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
    expect_list_or_string(
        lhs,
        lhs_span,
        TypeMismatchContext::BinaryConcat { side: Side::Lhs },
    )?;
    expect_list_or_string(
        rhs,
        rhs_span,
        TypeMismatchContext::BinaryConcat { side: Side::Rhs },
    )?;
    match (lhs, rhs) {
        (AnalyzedType::Dynamic, _) | (_, AnalyzedType::Dynamic) => Ok(AnalyzedType::Dynamic),
        (AnalyzedType::Resolved(GqlType::String), AnalyzedType::Resolved(GqlType::String)) => {
            Ok(AnalyzedType::Resolved(GqlType::String))
        }
        (
            AnalyzedType::Resolved(GqlType::List(lhs_inner)),
            AnalyzedType::Resolved(GqlType::List(rhs_inner)),
        ) => meet_gql_types(lhs_inner, rhs_inner)
            .map(|inner| AnalyzedType::Resolved(GqlType::List(Box::new(inner))))
            .ok_or_else(|| {
                type_mismatch(
                    TypeMismatchContext::BinaryConcat { side: Side::Rhs },
                    ExpectedType::Specific(GqlType::List(lhs_inner.clone())),
                    GqlType::List(rhs_inner.clone()),
                    rhs_span,
                )
            }),
        (AnalyzedType::Resolved(lhs_ty), AnalyzedType::Resolved(rhs_ty)) => Err(type_mismatch(
            TypeMismatchContext::BinaryConcat { side: Side::Rhs },
            ExpectedType::Specific(lhs_ty.clone()),
            rhs_ty.clone(),
            rhs_span,
        )),
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

fn expect_numeric(
    ty: &AnalyzedType,
    span: SourceSpan,
    context: TypeMismatchContext,
) -> Result<(), AnalysisError> {
    match ty {
        // A NULL operand is accepted: ISO three-valued logic makes the operator
        // yield NULL rather than a type error (the runtime already does so).
        AnalyzedType::Dynamic | AnalyzedType::Resolved(GqlType::Null) => Ok(()),
        AnalyzedType::Resolved(found) if is_numeric(found) => Ok(()),
        AnalyzedType::Resolved(found) => Err(type_mismatch(
            context,
            ExpectedType::Numeric,
            found.clone(),
            span,
        )),
    }
}

fn expect_boolean(
    ty: &AnalyzedType,
    span: SourceSpan,
    context: TypeMismatchContext,
) -> Result<(), AnalysisError> {
    match ty {
        // A NULL operand is accepted: under three-valued logic a boolean
        // operator over NULL yields NULL, not a type error (runtime parity).
        AnalyzedType::Dynamic
        | AnalyzedType::Resolved(GqlType::Boolean)
        | AnalyzedType::Resolved(GqlType::Null) => Ok(()),
        AnalyzedType::Resolved(found) => Err(type_mismatch(
            context,
            ExpectedType::Boolean,
            found.clone(),
            span,
        )),
    }
}

fn expect_string(
    ty: &AnalyzedType,
    span: SourceSpan,
    context: TypeMismatchContext,
) -> Result<(), AnalysisError> {
    match ty {
        AnalyzedType::Dynamic
        | AnalyzedType::Resolved(GqlType::String)
        | AnalyzedType::Resolved(GqlType::Null) => Ok(()),
        AnalyzedType::Resolved(found) => Err(type_mismatch(
            context,
            ExpectedType::String,
            found.clone(),
            span,
        )),
    }
}

fn expect_comparable(
    ty: &AnalyzedType,
    span: SourceSpan,
    context: TypeMismatchContext,
) -> Result<(), AnalysisError> {
    match ty {
        // A NULL operand is accepted: an ordered comparison against NULL yields
        // NULL under three-valued logic, not a type error (runtime parity).
        AnalyzedType::Dynamic | AnalyzedType::Resolved(GqlType::Null) => Ok(()),
        AnalyzedType::Resolved(found) if comparable_family(found).is_some() => Ok(()),
        AnalyzedType::Resolved(found) => Err(type_mismatch(
            context,
            ExpectedType::Comparable,
            found.clone(),
            span,
        )),
    }
}

fn expect_list_or_string(
    ty: &AnalyzedType,
    span: SourceSpan,
    context: TypeMismatchContext,
) -> Result<(), AnalysisError> {
    match ty {
        AnalyzedType::Dynamic
        | AnalyzedType::Resolved(GqlType::String)
        | AnalyzedType::Resolved(GqlType::List(_)) => Ok(()),
        AnalyzedType::Resolved(found) => Err(type_mismatch(
            context,
            ExpectedType::ListOrString,
            found.clone(),
            span,
        )),
    }
}

fn ensure_same_comparable_family(
    lhs: &AnalyzedType,
    rhs: &AnalyzedType,
    rhs_span: SourceSpan,
    context: TypeMismatchContext,
) -> Result<(), AnalysisError> {
    if let (AnalyzedType::Resolved(lhs_ty), AnalyzedType::Resolved(rhs_ty)) = (lhs, rhs)
        // A NULL operand has no comparable family and never forms a mismatch:
        // the comparison evaluates to NULL under three-valued logic regardless
        // of the other side's family (e.g. `NULL < 5` is valid, yields NULL).
        && !matches!(lhs_ty, GqlType::Null)
        && !matches!(rhs_ty, GqlType::Null)
        && comparable_family(lhs_ty) != comparable_family(rhs_ty)
    {
        return Err(type_mismatch(
            context,
            ExpectedType::Specific(lhs_ty.clone()),
            rhs_ty.clone(),
            rhs_span,
        ));
    }
    Ok(())
}

fn meet_gql_types(lhs: &GqlType, rhs: &GqlType) -> Option<GqlType> {
    if lhs == rhs {
        return Some(lhs.clone());
    }
    if matches!(lhs, GqlType::Null) {
        return Some(rhs.clone());
    }
    if matches!(rhs, GqlType::Null) {
        return Some(lhs.clone());
    }
    if is_numeric(lhs) && is_numeric(rhs) {
        return numeric_promotion(lhs, rhs);
    }
    match (lhs, rhs) {
        (GqlType::List(lhs_inner), GqlType::List(rhs_inner)) => {
            meet_gql_types(lhs_inner, rhs_inner).map(|ty| GqlType::List(Box::new(ty)))
        }
        _ => None,
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

fn is_supported_typed_target(ty: &GqlType) -> bool {
    match ty {
        GqlType::String
        | GqlType::Boolean
        | GqlType::Integer
        | GqlType::Float
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::Int128
        | GqlType::Uint8
        | GqlType::Uint16
        | GqlType::Uint32
        | GqlType::Uint64
        | GqlType::Uint128
        | GqlType::SmallInt
        | GqlType::BigInt
        | GqlType::Decimal
        | GqlType::Float32
        | GqlType::Float64
        | GqlType::Bytes
        | GqlType::Uuid
        | GqlType::ZonedDateTime
        | GqlType::LocalDateTime
        | GqlType::Date
        | GqlType::ZonedTime
        | GqlType::LocalTime
        | GqlType::Duration
        | GqlType::Path
        | GqlType::Null
        | GqlType::Nothing => true,
        GqlType::List(inner) => is_supported_typed_target(inner),
        // Why: per ISO 39075:2024 §18.9 <record type> + §19.6 <value type predicate>, a
        // record type is an authorized typed-predicate target. Closed-record field types
        // are validated recursively, mirroring the List(inner) arm above.
        GqlType::Record(RecordType::Open) => true,
        GqlType::Record(RecordType::Closed(fields)) => {
            fields.iter().all(|(_, ty)| is_supported_typed_target(ty))
        }
        GqlType::GraphRef | GqlType::NodeRef | GqlType::EdgeRef | GqlType::TableRef => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComparableFamily {
    Numeric,
    String,
    Bytes,
    Temporal,
}

fn comparable_family(ty: &GqlType) -> Option<ComparableFamily> {
    if is_numeric(ty) {
        return Some(ComparableFamily::Numeric);
    }
    Some(match ty {
        GqlType::String => ComparableFamily::String,
        GqlType::Bytes => ComparableFamily::Bytes,
        GqlType::ZonedDateTime
        | GqlType::LocalDateTime
        | GqlType::Date
        | GqlType::ZonedTime
        | GqlType::LocalTime
        | GqlType::Duration => ComparableFamily::Temporal,
        _ => return None,
    })
}

trait SpanMax {
    fn max(self, other: Self) -> Self;
}

impl SpanMax for SourceSpan {
    fn max(self, other: Self) -> Self {
        if self.byte_len == 0 { other } else { self }
    }
}

/// Infer the result type of an explicit `CAST(<value> AS <target_type>)`
/// expression.
///
/// The analyzer reports the cast's static result type as the declared target
/// type. Runtime validity (whether the source value can actually be cast to
/// the target) is enforced by `runtime::evaluator::cast::eval_cast` per ISO
/// §22; the analyzer does not pre-reject because (i) source types are often
/// Dynamic, (ii) ISO §22 specifies a runtime error model (`22018`, `22003`,
/// `42N01`) rather than a compile-time rejection model.
pub(crate) fn cast(target_type: &GqlType) -> Result<AnalyzedType, AnalysisError> {
    Ok(AnalyzedType::Resolved(target_type.clone()))
}
