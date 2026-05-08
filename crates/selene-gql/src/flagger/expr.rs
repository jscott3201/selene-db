//! Expression Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{
    ValueExpr,
    ast::expr::{IsCheckKind, Literal},
    error::ParserError,
};

use super::{check_feature, query};

pub(crate) fn value(value: &ValueExpr) -> Result<(), ParserError> {
    match value {
        ValueExpr::Literal(value) => literal(value),
        ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => Ok(()),
        ValueExpr::PropertyAccess { target, .. } => self::value(target),
        ValueExpr::ListAccess { target, index, .. } => {
            self::value(target)?;
            self::value(index)
        }
        ValueExpr::ListLiteral { items, .. } => values(items),
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                self::value(value)?;
            }
            Ok(())
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            self::value(lhs)?;
            self::value(rhs)
        }
        ValueExpr::UnaryOp { operand, .. } => self::value(operand),
        ValueExpr::FunctionCall { args, .. } => values(args),
        ValueExpr::IsCheck {
            operand,
            kind,
            span,
            ..
        } => {
            self::value(operand)?;
            is_check(kind, *span)
        }
        ValueExpr::InList { operand, list, .. } => {
            self::value(operand)?;
            values(list)
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            self::value(operand)?;
            self::value(pattern)
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            self::value(operand)?;
            self::value(low)?;
            self::value(high)
        }
        ValueExpr::AllDifferent { items, span } => {
            check_feature(FeatureId::G113, *span)?;
            values(items)
        }
        ValueExpr::Same { items, span } => {
            check_feature(FeatureId::G114, *span)?;
            values(items)
        }
        ValueExpr::PropertyExists { target, span, .. } => {
            check_feature(FeatureId::G115, *span)?;
            self::value(target)
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, result) in branches {
                self::value(condition)?;
                self::value(result)?;
            }
            if let Some(value) = else_branch {
                self::value(value)?;
            }
            Ok(())
        }
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            query::match_clause(pattern)
        }
    }
}

fn values(values: &[ValueExpr]) -> Result<(), ParserError> {
    for value in values {
        self::value(value)?;
    }
    Ok(())
}

fn literal(_value: &Literal) -> Result<(), ParserError> {
    Ok(())
}

fn is_check(kind: &IsCheckKind, span: crate::SourceSpan) -> Result<(), ParserError> {
    match kind {
        IsCheckKind::Null | IsCheckKind::TruthValue(_) | IsCheckKind::Typed(_) => Ok(()),
        IsCheckKind::Normalized(_) => Ok(()),
        IsCheckKind::Directed => check_feature(FeatureId::G110, span),
        IsCheckKind::Labeled(_) => {
            check_feature(FeatureId::G111, span)?;
            Ok(())
        }
        IsCheckKind::SourceOf(value) => {
            check_feature(FeatureId::G112, span)?;
            self::value(value)
        }
        IsCheckKind::DestinationOf(value) => {
            check_feature(FeatureId::G112, span)?;
            self::value(value)
        }
    }
}
