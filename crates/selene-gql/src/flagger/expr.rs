//! Expression Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{
    ValueExpr,
    ast::{
        expr::{BinaryOp, IsCheckKind, Literal},
        types::{GqlType, RecordType},
    },
};

use super::{FeatureUse, query, record_feature};

pub(crate) fn value(value: &ValueExpr, uses: &mut Vec<FeatureUse>) {
    match value {
        ValueExpr::Literal(value) => literal(value, uses),
        ValueExpr::Variable { .. } => {}
        ValueExpr::Parameter { span, .. } => {
            record_feature(uses, FeatureId::GE04, *span);
            record_feature(uses, FeatureId::GE05, *span);
        }
        ValueExpr::PropertyAccess { target, .. } => self::value(target, uses),
        ValueExpr::ListAccess { target, index, .. } => {
            self::value(target, uses);
            self::value(index, uses);
        }
        ValueExpr::ListLiteral { items, span } => {
            record_feature(uses, FeatureId::GV50, *span);
            values(items, uses);
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                self::value(value, uses);
            }
        }
        ValueExpr::BinaryOp { op, lhs, rhs, span } => {
            if matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Power
            ) {
                record_feature(uses, FeatureId::GA01, *span);
            }
            if *op == BinaryOp::Xor {
                record_feature(uses, FeatureId::GE07, *span);
            }
            self::value(lhs, uses);
            self::value(rhs, uses);
        }
        ValueExpr::UnaryOp { operand, .. } => self::value(operand, uses),
        ValueExpr::FunctionCall {
            name, args, span, ..
        } => {
            if name.len() == 1 && name.first().as_str().eq_ignore_ascii_case("size") {
                record_feature(uses, FeatureId::GF13, *span);
            }
            values(args, uses);
        }
        ValueExpr::IsCheck {
            operand,
            kind,
            span,
            ..
        } => {
            self::value(operand, uses);
            is_check(kind, *span, uses);
        }
        ValueExpr::InList { operand, list, .. } => {
            self::value(operand, uses);
            values(list, uses);
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            self::value(operand, uses);
            self::value(pattern, uses);
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            self::value(operand, uses);
            self::value(low, uses);
            self::value(high, uses);
        }
        ValueExpr::AllDifferent { items, span } => {
            record_feature(uses, FeatureId::G113, *span);
            values(items, uses);
        }
        ValueExpr::Same { items, span } => {
            record_feature(uses, FeatureId::G114, *span);
            values(items, uses);
        }
        ValueExpr::PropertyExists { target, span, .. } => {
            record_feature(uses, FeatureId::G115, *span);
            self::value(target, uses);
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, result) in branches {
                self::value(condition, uses);
                self::value(result, uses);
            }
            if let Some(value) = else_branch {
                self::value(value, uses);
            }
        }
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            query::match_clause(pattern, uses);
        }
        ValueExpr::ValueSubquery { body, .. } => {
            query::query_pipeline(body, uses);
        }
    }
}

fn values(values: &[ValueExpr], uses: &mut Vec<FeatureUse>) {
    for value in values {
        self::value(value, uses);
    }
}

fn literal(value: &Literal, uses: &mut Vec<FeatureUse>) {
    match value {
        Literal::Float(_, span) => record_feature(uses, FeatureId::GA01, *span),
        Literal::String(_, _) | Literal::Bool(_, _) | Literal::Integer(_, _) | Literal::Null(_) => {
        }
    }
}

fn is_check(kind: &IsCheckKind, span: crate::SourceSpan, uses: &mut Vec<FeatureUse>) {
    match kind {
        IsCheckKind::Null | IsCheckKind::TruthValue(_) => {}
        IsCheckKind::Typed(ty) => gql_type(ty, span, uses),
        IsCheckKind::Normalized(_) => {}
        IsCheckKind::Directed => record_feature(uses, FeatureId::G110, span),
        IsCheckKind::Labeled(_) => {
            record_feature(uses, FeatureId::G111, span);
        }
        IsCheckKind::SourceOf(value) => {
            record_feature(uses, FeatureId::G112, span);
            self::value(value, uses);
        }
        IsCheckKind::DestinationOf(value) => {
            record_feature(uses, FeatureId::G112, span);
            self::value(value, uses);
        }
    }
}

pub(crate) fn gql_type(ty: &GqlType, span: crate::SourceSpan, uses: &mut Vec<FeatureUse>) {
    match ty {
        GqlType::String | GqlType::Boolean | GqlType::Integer | GqlType::Float => {}
        GqlType::Uint8 => {
            record_feature(uses, FeatureId::GV01, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int8 => {
            record_feature(uses, FeatureId::GV02, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Uint16 => {
            record_feature(uses, FeatureId::GV03, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int16 => {
            record_feature(uses, FeatureId::GV04, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::SmallInt => {
            record_feature(uses, FeatureId::GV05, span);
            record_feature(uses, FeatureId::GV18, span);
        }
        GqlType::Uint32 => {
            record_feature(uses, FeatureId::GV06, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int32 => {
            record_feature(uses, FeatureId::GV07, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Uint64 => {
            record_feature(uses, FeatureId::GV08, span);
            record_feature(uses, FeatureId::GV11, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::BigInt => {
            record_feature(uses, FeatureId::GV10, span);
            record_feature(uses, FeatureId::GV19, span);
        }
        GqlType::Int64 => {
            record_feature(uses, FeatureId::GV12, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Uint128 => {
            record_feature(uses, FeatureId::GV13, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int128 => {
            record_feature(uses, FeatureId::GV14, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Decimal => record_feature(uses, FeatureId::GV17, span),
        GqlType::Float32 => record_feature(uses, FeatureId::GV21, span),
        GqlType::Float64 => record_feature(uses, FeatureId::GV24, span),
        GqlType::Bytes | GqlType::Binary | GqlType::VarBinary => {
            record_feature(uses, FeatureId::GV35, span);
        }
        GqlType::Date | GqlType::LocalDateTime | GqlType::LocalTime => {
            record_feature(uses, FeatureId::GV39, span);
        }
        GqlType::ZonedDateTime | GqlType::ZonedTime => {
            record_feature(uses, FeatureId::GV40, span);
        }
        GqlType::Duration => record_feature(uses, FeatureId::GV41, span),
        GqlType::Record(record) => {
            record_feature(uses, FeatureId::GV45, span);
            match record {
                RecordType::Open => record_feature(uses, FeatureId::GV47, span),
                RecordType::Closed(fields) => {
                    record_feature(uses, FeatureId::GV46, span);
                    for (_, ty) in fields {
                        if matches!(ty, GqlType::Record(_)) {
                            record_feature(uses, FeatureId::GV48, span);
                        }
                        gql_type(ty, span, uses);
                    }
                }
            }
        }
        GqlType::List(inner) => {
            record_feature(uses, FeatureId::GV50, span);
            gql_type(inner, span, uses);
        }
        GqlType::Path => record_feature(uses, FeatureId::GV55, span),
        GqlType::GraphRef => record_feature(uses, FeatureId::GV60, span),
        GqlType::TableRef => record_feature(uses, FeatureId::GV61, span),
        GqlType::NodeRef | GqlType::EdgeRef | GqlType::Null | GqlType::Nothing => {}
    }
}
