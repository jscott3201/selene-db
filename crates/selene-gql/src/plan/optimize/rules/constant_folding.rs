//! Literal constant-folding rule.

use selene_core::DbString;
use unicode_normalization::UnicodeNormalization;

use crate::{
    BinaryOp, DecimalLiteralKind, FloatLiteralKind, ImplDefinedCaps, Literal, SourceSpan, UnaryOp,
    ValueExpr,
    plan::{
        ExecutionPlan,
        optimize::{OptimizeContext, Rule, Transformed, walk},
    },
};

/// Fold literal-only value expressions while preserving surrounding type cells.
pub struct ConstantFolding;

impl Rule for ConstantFolding {
    fn name(&self) -> &'static str {
        "constant_folding"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let mut changed = walk::walk_value_exprs(&mut plan, &mut |expr| {
            if let Some(folded) = fold_expr(expr, ctx.impl_defined_caps) {
                *expr = folded;
                true
            } else {
                false
            }
        });
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn fold_expr(expr: &ValueExpr, caps: &ImplDefinedCaps) -> Option<ValueExpr> {
    match expr {
        ValueExpr::UnaryOp { op, operand, span } => fold_unary(*op, operand, *span),
        ValueExpr::BinaryOp { op, lhs, rhs, span } => fold_binary(*op, lhs, rhs, *span, caps),
        _ => None,
    }
}

fn integer_value(literal: &Literal) -> Option<i64> {
    match literal {
        Literal::Integer(value, _) | Literal::RadixInteger(value, _, _) => Some(*value),
        _ => None,
    }
}

fn fold_unary(op: UnaryOp, operand: &ValueExpr, span: SourceSpan) -> Option<ValueExpr> {
    let ValueExpr::Literal(literal) = operand else {
        return None;
    };
    match (op, literal) {
        (UnaryOp::Not, Literal::Bool(value, _)) => {
            Some(ValueExpr::Literal(Literal::Bool(!value, span)))
        }
        (UnaryOp::Negate, literal @ (Literal::Integer(_, _) | Literal::RadixInteger(_, _, _))) => {
            integer_value(literal)
                .and_then(i64::checked_neg)
                .map(|folded| ValueExpr::Literal(Literal::Integer(folded, span)))
        }
        (UnaryOp::Negate, Literal::Decimal(value, _, kind)) => {
            Some(ValueExpr::Literal(Literal::Decimal(-*value, span, *kind)))
        }
        (UnaryOp::Negate, Literal::Float(value, _, kind)) => finite_float(-value, span, *kind),
        (UnaryOp::Not, _)
        | (UnaryOp::Negate, Literal::Bool(_, _))
        | (UnaryOp::Negate, Literal::String(_, _))
        | (UnaryOp::Negate, Literal::Bytes(_, _))
        | (UnaryOp::Negate, Literal::Uuid(_, _))
        | (UnaryOp::Negate, Literal::ZonedDateTime(_, _))
        | (UnaryOp::Negate, Literal::LocalDateTime(_, _))
        | (UnaryOp::Negate, Literal::Date(_, _))
        | (UnaryOp::Negate, Literal::ZonedTime(_, _))
        | (UnaryOp::Negate, Literal::LocalTime(_, _))
        | (UnaryOp::Negate, Literal::Duration(_, _))
        | (UnaryOp::Negate, Literal::Null(_)) => None,
    }
}

fn fold_binary(
    op: BinaryOp,
    lhs: &ValueExpr,
    rhs: &ValueExpr,
    span: SourceSpan,
    caps: &ImplDefinedCaps,
) -> Option<ValueExpr> {
    let (ValueExpr::Literal(lhs), ValueExpr::Literal(rhs)) = (lhs, rhs) else {
        return None;
    };
    if matches!(lhs, Literal::Null(_)) || matches!(rhs, Literal::Null(_)) {
        return None;
    }
    match op {
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
            fold_arithmetic(op, lhs, rhs, span)
        }
        BinaryOp::Power => None,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            fold_comparison(op, lhs, rhs, span)
        }
        BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => fold_boolean(op, lhs, rhs, span),
        BinaryOp::Concat => fold_concat(lhs, rhs, span, caps),
        BinaryOp::Contains | BinaryOp::StartsWith | BinaryOp::EndsWith => None,
    }
}

fn fold_arithmetic(
    op: BinaryOp,
    lhs: &Literal,
    rhs: &Literal,
    span: SourceSpan,
) -> Option<ValueExpr> {
    match (lhs, rhs) {
        (left, right) if integer_value(left).is_some() && integer_value(right).is_some() => {
            let left = integer_value(left)?;
            let right = integer_value(right)?;
            let folded = match op {
                BinaryOp::Add => left.checked_add(right),
                BinaryOp::Sub => left.checked_sub(right),
                BinaryOp::Mul => left.checked_mul(right),
                BinaryOp::Div => (right != 0).then(|| left.checked_div(right)).flatten(),
                BinaryOp::Mod => (right != 0).then(|| left.checked_rem(right)).flatten(),
                _ => None,
            }?;
            Some(ValueExpr::Literal(Literal::Integer(folded, span)))
        }
        (Literal::Float(left, _, _), Literal::Float(right, _, _)) => {
            let folded = match op {
                BinaryOp::Add => left + right,
                BinaryOp::Sub => left - right,
                BinaryOp::Mul => left * right,
                BinaryOp::Div if *right != 0.0 => left / right,
                BinaryOp::Mod if *right != 0.0 => left % right,
                _ => return None,
            };
            finite_float(
                folded,
                span,
                FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
            )
        }
        (Literal::Decimal(left, _, _), Literal::Decimal(right, _, _)) => {
            let folded = match op {
                BinaryOp::Add => left.checked_add(*right),
                BinaryOp::Sub => left.checked_sub(*right),
                BinaryOp::Mul => left.checked_mul(*right),
                BinaryOp::Div => left.checked_div(*right),
                BinaryOp::Mod => left.checked_rem(*right),
                _ => return None,
            }?;
            Some(ValueExpr::Literal(Literal::Decimal(
                folded,
                span,
                DecimalLiteralKind::CommonWithoutSuffix,
            )))
        }
        _ => None,
    }
}

fn fold_comparison(
    op: BinaryOp,
    lhs: &Literal,
    rhs: &Literal,
    span: SourceSpan,
) -> Option<ValueExpr> {
    let folded = match (lhs, rhs) {
        (Literal::Bool(left, _), Literal::Bool(right, _)) => match op {
            BinaryOp::Eq => left == right,
            BinaryOp::Ne => left != right,
            _ => return None,
        },
        (left, right) if integer_value(left).is_some() && integer_value(right).is_some() => {
            compare_ordering(op, &integer_value(left)?, &integer_value(right)?)
        }
        (Literal::Float(left, _, _), Literal::Float(right, _, _))
            if left.is_finite() && right.is_finite() =>
        {
            compare_partial(op, *left, *right)?
        }
        (left, Literal::Float(right, _, _))
            if right.is_finite() && integer_value(left).is_some() =>
        {
            compare_partial(op, integer_value(left)? as f64, *right)?
        }
        (Literal::Float(left, _, _), right)
            if left.is_finite() && integer_value(right).is_some() =>
        {
            compare_partial(op, *left, integer_value(right)? as f64)?
        }
        (Literal::Decimal(left, _, _), Literal::Decimal(right, _, _)) => {
            compare_ordering(op, left, right)
        }
        (Literal::String(left, _), Literal::String(right, _)) => {
            compare_ordering(op, left.as_str(), right.as_str())
        }
        (Literal::Bytes(left, _), Literal::Bytes(right, _)) => {
            compare_ordering(op, left.as_ref(), right.as_ref())
        }
        (Literal::Date(left, _), Literal::Date(right, _)) => compare_ordering(op, left, right),
        (Literal::LocalDateTime(left, _), Literal::LocalDateTime(right, _)) => {
            compare_ordering(op, left, right)
        }
        (Literal::LocalTime(left, _), Literal::LocalTime(right, _)) => {
            compare_ordering(op, left, right)
        }
        _ => return None,
    };
    Some(ValueExpr::Literal(Literal::Bool(folded, span)))
}

fn fold_boolean(op: BinaryOp, lhs: &Literal, rhs: &Literal, span: SourceSpan) -> Option<ValueExpr> {
    let (Literal::Bool(left, _), Literal::Bool(right, _)) = (lhs, rhs) else {
        return None;
    };
    let folded = match op {
        BinaryOp::And => *left && *right,
        BinaryOp::Or => *left || *right,
        BinaryOp::Xor => *left ^ *right,
        _ => return None,
    };
    Some(ValueExpr::Literal(Literal::Bool(folded, span)))
}

fn fold_concat(
    lhs: &Literal,
    rhs: &Literal,
    span: SourceSpan,
    caps: &ImplDefinedCaps,
) -> Option<ValueExpr> {
    match (lhs, rhs) {
        (Literal::String(left, _), Literal::String(right, _)) => {
            let value = folded_string_concat(left.as_str(), right.as_str(), caps)?;
            let db_string_value = DbString::from_string(value).ok()?;
            Some(ValueExpr::Literal(Literal::String(db_string_value, span)))
        }
        (Literal::Bytes(left, _), Literal::Bytes(right, _)) => {
            let value = folded_byte_concat(left, right, caps)?;
            Some(ValueExpr::Literal(Literal::Bytes(
                value.into_boxed_slice().into(),
                span,
            )))
        }
        _ => None,
    }
}

fn folded_string_concat(lhs: &str, rhs: &str, caps: &ImplDefinedCaps) -> Option<String> {
    let byte_len = lhs.len().checked_add(rhs.len())?;
    let mut value = String::with_capacity(byte_len);
    value.push_str(lhs);
    value.push_str(rhs);
    if unicode_normalization::is_nfc(lhs) && unicode_normalization::is_nfc(rhs) {
        value = value.nfc().collect();
    }
    let char_count = value.chars().count();
    let max_chars = usize::try_from(caps.max_string_length).unwrap_or(usize::MAX);
    if char_count <= max_chars {
        return Some(value);
    }
    let overflow_chars = char_count - max_chars;
    value
        .chars()
        .rev()
        .take(overflow_chars)
        .all(char::is_whitespace)
        .then(|| value.chars().take(max_chars).collect())
}

fn folded_byte_concat(lhs: &[u8], rhs: &[u8], caps: &ImplDefinedCaps) -> Option<Vec<u8>> {
    let total_len = lhs.len().checked_add(rhs.len())?;
    let max_len = usize::try_from(caps.max_byte_string_length).unwrap_or(usize::MAX);
    let output_len = if total_len <= max_len {
        total_len
    } else {
        let overflow = total_len - max_len;
        byte_suffix_is_zero(lhs, rhs, overflow).then_some(max_len)?
    };
    let mut value = Vec::with_capacity(output_len);
    if output_len <= lhs.len() {
        value.extend_from_slice(&lhs[..output_len]);
    } else {
        value.extend_from_slice(lhs);
        value.extend_from_slice(&rhs[..output_len - lhs.len()]);
    }
    Some(value)
}

fn byte_suffix_is_zero(lhs: &[u8], rhs: &[u8], suffix_len: usize) -> bool {
    if suffix_len <= rhs.len() {
        return rhs[rhs.len() - suffix_len..].iter().all(|byte| *byte == 0);
    }
    rhs.iter().all(|byte| *byte == 0)
        && lhs[lhs.len() - (suffix_len - rhs.len())..]
            .iter()
            .all(|byte| *byte == 0)
}

fn finite_float(value: f64, span: SourceSpan, kind: FloatLiteralKind) -> Option<ValueExpr> {
    value
        .is_finite()
        .then_some(ValueExpr::Literal(Literal::Float(value, span, kind)))
}

fn compare_ordering<T: Ord>(op: BinaryOp, lhs: T, rhs: T) -> bool {
    match op {
        BinaryOp::Eq => lhs == rhs,
        BinaryOp::Ne => lhs != rhs,
        BinaryOp::Lt => lhs < rhs,
        BinaryOp::Le => lhs <= rhs,
        BinaryOp::Gt => lhs > rhs,
        BinaryOp::Ge => lhs >= rhs,
        _ => false,
    }
}

fn compare_partial(op: BinaryOp, lhs: f64, rhs: f64) -> Option<bool> {
    Some(match op {
        BinaryOp::Eq => lhs == rhs,
        BinaryOp::Ne => lhs != rhs,
        BinaryOp::Lt => lhs < rhs,
        BinaryOp::Le => lhs <= rhs,
        BinaryOp::Gt => lhs > rhs,
        BinaryOp::Ge => lhs >= rhs,
        _ => return None,
    })
}
