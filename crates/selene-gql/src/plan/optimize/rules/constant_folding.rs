//! Literal constant-folding rule.

use selene_core::intern_with_admission;

use crate::{
    BinaryOp, Literal, SourceSpan, UnaryOp, ValueExpr,
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
            if let Some(folded) = fold_expr(expr) {
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

fn fold_expr(expr: &ValueExpr) -> Option<ValueExpr> {
    match expr {
        ValueExpr::UnaryOp { op, operand, span } => fold_unary(*op, operand, *span),
        ValueExpr::BinaryOp { op, lhs, rhs, span } => fold_binary(*op, lhs, rhs, *span),
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
        (UnaryOp::Negate, Literal::Integer(value, _)) => value
            .checked_neg()
            .map(|folded| ValueExpr::Literal(Literal::Integer(folded, span))),
        (UnaryOp::Negate, Literal::Float(value, _)) => finite_float(-value, span),
        (UnaryOp::Not, _)
        | (UnaryOp::Negate, Literal::Bool(_, _))
        | (UnaryOp::Negate, Literal::String(_, _))
        | (UnaryOp::Negate, Literal::Uuid(_, _))
        | (UnaryOp::Negate, Literal::Null(_)) => None,
    }
}

fn fold_binary(
    op: BinaryOp,
    lhs: &ValueExpr,
    rhs: &ValueExpr,
    span: SourceSpan,
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
        BinaryOp::Concat => fold_concat(lhs, rhs, span),
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
        (Literal::Integer(left, _), Literal::Integer(right, _)) => {
            let folded = match op {
                BinaryOp::Add => left.checked_add(*right),
                BinaryOp::Sub => left.checked_sub(*right),
                BinaryOp::Mul => left.checked_mul(*right),
                BinaryOp::Div => (*right != 0).then(|| left.checked_div(*right)).flatten(),
                BinaryOp::Mod => (*right != 0).then(|| left.checked_rem(*right)).flatten(),
                _ => None,
            }?;
            Some(ValueExpr::Literal(Literal::Integer(folded, span)))
        }
        (Literal::Float(left, _), Literal::Float(right, _)) => {
            let folded = match op {
                BinaryOp::Add => left + right,
                BinaryOp::Sub => left - right,
                BinaryOp::Mul => left * right,
                BinaryOp::Div if *right != 0.0 => left / right,
                BinaryOp::Mod if *right != 0.0 => left % right,
                _ => return None,
            };
            finite_float(folded, span)
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
        (Literal::Integer(left, _), Literal::Integer(right, _)) => {
            compare_ordering(op, left, right)
        }
        (Literal::Float(left, _), Literal::Float(right, _))
            if left.is_finite() && right.is_finite() =>
        {
            compare_partial(op, *left, *right)?
        }
        (Literal::Integer(left, _), Literal::Float(right, _)) if right.is_finite() => {
            compare_partial(op, *left as f64, *right)?
        }
        (Literal::Float(left, _), Literal::Integer(right, _)) if left.is_finite() => {
            compare_partial(op, *left, *right as f64)?
        }
        (Literal::String(left, _), Literal::String(right, _)) => {
            compare_ordering(op, left.as_str(), right.as_str())
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

fn fold_concat(lhs: &Literal, rhs: &Literal, span: SourceSpan) -> Option<ValueExpr> {
    let (Literal::String(left, _), Literal::String(right, _)) = (lhs, rhs) else {
        return None;
    };
    let mut value = String::with_capacity(left.as_str().len() + right.as_str().len());
    value.push_str(left.as_str());
    value.push_str(right.as_str());
    let interned = intern_with_admission(&value).ok()?.0;
    Some(ValueExpr::Literal(Literal::String(interned, span)))
}

fn finite_float(value: f64, span: SourceSpan) -> Option<ValueExpr> {
    value
        .is_finite()
        .then_some(ValueExpr::Literal(Literal::Float(value, span)))
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
