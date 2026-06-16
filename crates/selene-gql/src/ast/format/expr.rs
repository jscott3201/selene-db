//! Read-side `ValueExpr` formatting for the AST pretty-printer.

use std::fmt::{self, Write as _};

use super::super::format_ident::{escape_string, fmt_call_segment, fmt_expr_ident, fmt_ident};
use super::super::{
    DecimalLiteralKind, ExistsBody, FloatLiteralKind, IntegerLiteralKind,
    TemporalDurationQualifier, UnaryOp, ValueExpr,
};
use super::is_check::{fmt_is_check, fmt_normal_form};
use super::keywords::fmt_binary;
use super::{cast, fmt_match, fmt_parameter, fmt_pipeline, trim};

pub(super) fn fmt_expr(out: &mut String, expr: &ValueExpr) -> fmt::Result {
    match expr {
        ValueExpr::Literal(literal) => match literal {
            crate::Literal::Bool(value, _) => out.push_str(if *value { "true" } else { "false" }),
            crate::Literal::Integer(value, _) => write!(out, "{value}")?,
            crate::Literal::RadixInteger(value, _, kind) => fmt_radix_integer(out, *value, *kind)?,
            crate::Literal::Decimal(value, _, kind) => {
                fmt_decimal_literal(out, value, *kind)?;
            }
            crate::Literal::Float(value, _, kind) => fmt_float_literal(out, *value, *kind)?,
            crate::Literal::String(value, _, _) => {
                write!(out, "'{}'", escape_string(value.as_str()))?;
            }
            crate::Literal::Bytes(value, _) => {
                out.push_str("X'");
                for byte in value.iter() {
                    write!(out, "{byte:02X}")?;
                }
                out.push('\'');
            }
            crate::Literal::Uuid(value, _, _) => write!(out, "UUID '{value}'")?,
            crate::Literal::ZonedDateTime(value, _, _) => {
                write!(out, "ZONED DATETIME '{}'", format_zoned_datetime(value))?;
            }
            crate::Literal::LocalDateTime(value, _, _) => write!(out, "LOCAL DATETIME '{value}'")?,
            crate::Literal::Date(value, _, _) => write!(out, "DATE '{value}'")?,
            crate::Literal::ZonedTime(value, _, _) => {
                write!(out, "ZONED TIME '{}'", format_zoned_time(value))?;
            }
            crate::Literal::LocalTime(value, _, _) => write!(out, "LOCAL TIME '{value}'")?,
            crate::Literal::Duration(value, _, _) => write!(out, "DURATION '{value}'")?,
            crate::Literal::Null(_) => out.push_str("null"),
        },
        ValueExpr::Variable { name, .. } => out.push_str(&fmt_expr_ident(name.clone())),
        ValueExpr::Parameter {
            name,
            declared_type,
            ..
        } => fmt_parameter(out, name.clone(), declared_type.as_ref())?,
        ValueExpr::PropertyAccess { target, key, .. } => {
            fmt_expr(out, target)?;
            write!(out, ".{}", fmt_ident(key.clone()))?;
        }
        ValueExpr::ListLiteral { items, .. } => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                fmt_expr(out, item)?;
            }
            out.push(']');
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            out.push('{');
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write!(out, "{}: ", fmt_ident(key.clone()))?;
                fmt_expr(out, value)?;
            }
            out.push('}');
        }
        ValueExpr::PathConstructor { elements, .. } => {
            out.push_str("PATH[");
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                fmt_expr(out, element)?;
            }
            out.push(']');
        }
        ValueExpr::BinaryOp { op, lhs, rhs, .. } => {
            // `MOD` and `POWER` are runtime-only operators with no infix
            // spelling in ISO GQL; round-trip them through their scalar
            // function form so the formatter output re-parses (the grammar
            // emits neither `%` nor `^`).
            if let Some(func) = match op {
                crate::ast::BinaryOp::Mod => Some("MOD"),
                crate::ast::BinaryOp::Power => Some("POWER"),
                _ => None,
            } {
                out.push_str(func);
                out.push('(');
                fmt_expr(out, lhs)?;
                out.push_str(", ");
                fmt_expr(out, rhs)?;
                out.push(')');
            } else {
                out.push('(');
                fmt_expr(out, lhs)?;
                write!(out, " {} ", fmt_binary(*op))?;
                fmt_expr(out, rhs)?;
                out.push(')');
            }
        }
        ValueExpr::UnaryOp { op, operand, .. } => {
            out.push('(');
            out.push_str(match op {
                UnaryOp::Negate => "-",
                UnaryOp::Not => "NOT ",
            });
            fmt_expr(out, operand)?;
            out.push(')');
        }
        ValueExpr::FunctionCall {
            name,
            args,
            star,
            distinct,
            ..
        } => {
            if let Some(keyword) = niladic_current_datetime_keyword(name, args, *star, *distinct) {
                out.push_str(keyword);
                return Ok(());
            }
            if let Some(keyword) = keyword_function_name(name) {
                out.push_str(keyword);
            } else {
                for (index, part) in name.iter().enumerate() {
                    if index > 0 {
                        out.push('.');
                    }
                    out.push_str(&fmt_call_segment(part.clone()));
                }
            }
            out.push('(');
            if *distinct {
                out.push_str("DISTINCT ");
            }
            if *star {
                out.push('*');
            } else {
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    fmt_expr(out, arg)?;
                }
            }
            out.push(')');
        }
        ValueExpr::DurationBetween {
            start,
            end,
            qualifier,
            ..
        } => {
            out.push_str("DURATION_BETWEEN(");
            fmt_expr(out, start)?;
            out.push_str(", ");
            fmt_expr(out, end)?;
            out.push_str(") ");
            out.push_str(fmt_temporal_duration_qualifier(*qualifier));
        }
        ValueExpr::Normalize { source, form, .. } => {
            out.push_str("NORMALIZE(");
            fmt_expr(out, source)?;
            if let Some(form) = form {
                out.push_str(", ");
                out.push_str(fmt_normal_form(*form));
            }
            out.push(')');
        }
        ValueExpr::Trim {
            spec,
            character,
            source,
            ..
        } => trim::fmt_trim_expr(out, *spec, character.as_deref(), source)?,
        ValueExpr::IsCheck {
            operand,
            kind,
            negated,
            ..
        } => fmt_is_check(out, operand, kind, *negated)?,
        ValueExpr::InList {
            operand,
            list,
            negated,
            ..
        } => {
            fmt_expr(out, operand)?;
            if *negated {
                out.push_str(" NOT");
            }
            out.push_str(" IN [");
            for (index, item) in list.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                fmt_expr(out, item)?;
            }
            out.push(']');
        }
        ValueExpr::InListExpression {
            operand,
            list,
            negated,
            ..
        } => {
            fmt_expr(out, operand)?;
            if *negated {
                out.push_str(" NOT");
            }
            out.push_str(" IN ");
            fmt_expr(out, list)?;
        }
        ValueExpr::AllDifferent { items, .. } => fmt_variadic(out, "ALL_DIFFERENT", items)?,
        ValueExpr::Same { items, .. } => fmt_variadic(out, "SAME", items)?,
        ValueExpr::PropertyExists { target, key, .. } => {
            out.push_str("PROPERTY_EXISTS(");
            fmt_expr(out, target)?;
            write!(out, ", '{}')", escape_string(key.as_str()))?;
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            out.push_str("CASE");
            for (condition, result) in branches {
                out.push_str(" WHEN ");
                fmt_expr(out, condition)?;
                out.push_str(" THEN ");
                fmt_expr(out, result)?;
            }
            if let Some(value) = else_branch {
                out.push_str(" ELSE ");
                fmt_expr(out, value)?;
            }
            out.push_str(" END");
        }
        ValueExpr::Exists { body, negated, .. } => {
            if *negated {
                out.push_str("NOT ");
            }
            out.push_str("EXISTS { ");
            match body {
                ExistsBody::Match(pattern) => fmt_match(out, pattern)?,
                ExistsBody::Query(pipeline) => fmt_pipeline(out, pipeline)?,
            }
            out.push_str(" }");
        }
        ValueExpr::ValueSubquery { body, .. } => {
            out.push_str("VALUE { ");
            fmt_pipeline(out, body)?;
            out.push_str(" }");
        }
        ValueExpr::Cast {
            value, target_type, ..
        } => cast::fmt_cast(out, value, target_type)?,
    }
    Ok(())
}

fn fmt_decimal_literal(
    out: &mut String,
    value: &rust_decimal::Decimal,
    kind: DecimalLiteralKind,
) -> fmt::Result {
    match kind {
        DecimalLiteralKind::CommonWithoutSuffix => {
            write!(out, "{value}")?;
            if !value.to_string().contains('.') {
                out.push('.');
            }
        }
        DecimalLiteralKind::CommonOrIntegerWithSuffix => write!(out, "{value}M")?,
        DecimalLiteralKind::ScientificWithSuffix => write!(out, "{value}E0M")?,
    }
    Ok(())
}

fn fmt_float_literal(out: &mut String, value: f64, kind: FloatLiteralKind) -> fmt::Result {
    match kind {
        FloatLiteralKind::ScientificWithoutSuffix => write!(out, "{value:e}")?,
        FloatLiteralKind::CommonOrIntegerWithFloatSuffix => write!(out, "{value}F")?,
        FloatLiteralKind::CommonOrIntegerWithDoubleSuffix => write!(out, "{value}D")?,
        FloatLiteralKind::ScientificWithFloatSuffix => write!(out, "{value:e}F")?,
        FloatLiteralKind::ScientificWithDoubleSuffix => write!(out, "{value:e}D")?,
    }
    Ok(())
}

fn fmt_radix_integer(out: &mut String, value: i64, kind: IntegerLiteralKind) -> fmt::Result {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = if value < 0 {
        -(value as i128)
    } else {
        value as i128
    };
    match kind {
        IntegerLiteralKind::Hexadecimal => write!(out, "{sign}0x{magnitude:X}"),
        IntegerLiteralKind::Octal => write!(out, "{sign}0o{magnitude:o}"),
        IntegerLiteralKind::Binary => write!(out, "{sign}0b{magnitude:b}"),
    }
}

fn fmt_temporal_duration_qualifier(qualifier: TemporalDurationQualifier) -> &'static str {
    match qualifier {
        TemporalDurationQualifier::YearToMonth => "YEAR TO MONTH",
        TemporalDurationQualifier::DayToSecond => "DAY TO SECOND",
    }
}

fn format_zoned_datetime(value: &jiff::Zoned) -> String {
    format!("{}{}", value.datetime(), value.offset())
}

fn format_zoned_time(value: &jiff::Zoned) -> String {
    format!("{}{}", value.time(), value.offset())
}

fn keyword_function_name(name: &crate::NonEmpty<selene_core::DbString>) -> Option<&'static str> {
    if name.len() != 1 {
        return None;
    }
    let segment = name.first().as_str();
    if segment.eq_ignore_ascii_case("elements") {
        Some("ELEMENTS")
    } else if segment.eq_ignore_ascii_case("labels") {
        Some("LABELS")
    } else {
        None
    }
}

fn niladic_current_datetime_keyword(
    name: &crate::NonEmpty<selene_core::DbString>,
    args: &[ValueExpr],
    star: bool,
    distinct: bool,
) -> Option<&'static str> {
    if star || distinct || !args.is_empty() || name.len() != 1 {
        return None;
    }
    let segment = name.first().as_str();
    if segment.eq_ignore_ascii_case("current_date") {
        Some("CURRENT_DATE")
    } else if segment.eq_ignore_ascii_case("current_time") {
        Some("CURRENT_TIME")
    } else if segment.eq_ignore_ascii_case("current_timestamp") {
        Some("CURRENT_TIMESTAMP")
    } else if segment.eq_ignore_ascii_case("local_time") {
        Some("LOCAL_TIME")
    } else {
        None
    }
}

fn fmt_variadic(out: &mut String, name: &str, items: &[ValueExpr]) -> fmt::Result {
    out.push_str(name);
    out.push('(');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        fmt_expr(out, item)?;
    }
    out.push(')');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::fmt_expr;
    use crate::ast::ValueExpr;
    use crate::ast::expr::{BinaryOp, Literal};
    use crate::ast::span::SourceSpan;

    fn int_lit(value: i64) -> ValueExpr {
        ValueExpr::Literal(Literal::Integer(value, SourceSpan::default()))
    }

    fn render(op: BinaryOp) -> String {
        let expr = ValueExpr::BinaryOp {
            op,
            lhs: Box::new(int_lit(2)),
            rhs: Box::new(int_lit(3)),
            span: SourceSpan::default(),
        };
        let mut out = String::new();
        fmt_expr(&mut out, &expr).expect("formats");
        out
    }

    #[test]
    fn power_and_mod_render_as_iso_function_form() {
        // `Power` and `Mod` are runtime-only operators backing ISO
        // `POWER(x, y)` / `MOD(x, y)`; the formatter must emit the function
        // form (never the non-ISO `^` / `%` infix) so output re-parses.
        assert_eq!(render(BinaryOp::Power), "POWER(2, 3)");
        assert_eq!(render(BinaryOp::Mod), "MOD(2, 3)");
    }
}
