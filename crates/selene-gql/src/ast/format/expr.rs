//! Read-side `ValueExpr` formatting for the AST pretty-printer.

use std::fmt::{self, Write as _};

use super::super::format_ident::{escape_string, fmt_call_segment, fmt_ident};
use super::super::{UnaryOp, ValueExpr};
use super::is_check::{fmt_is_check, fmt_normal_form};
use super::keywords::fmt_binary;
use super::{cast, fmt_match, fmt_parameter, fmt_pipeline, trim};

pub(super) fn fmt_expr(out: &mut String, expr: &ValueExpr) -> fmt::Result {
    match expr {
        ValueExpr::Literal(literal) => match literal {
            crate::Literal::Bool(value, _) => out.push_str(if *value { "true" } else { "false" }),
            crate::Literal::Integer(value, _) => write!(out, "{value}")?,
            crate::Literal::Float(value, _) => write!(out, "{value}")?,
            crate::Literal::String(value, _) => write!(out, "'{}'", escape_string(value.as_str()))?,
            crate::Literal::Uuid(value, _) => write!(out, "UUID '{value}'")?,
            crate::Literal::Null(_) => out.push_str("null"),
        },
        ValueExpr::Variable { name, .. } => out.push_str(&fmt_ident(name.clone())),
        ValueExpr::Parameter {
            name,
            declared_type,
            ..
        } => fmt_parameter(out, name.clone(), declared_type.as_ref())?,
        ValueExpr::PropertyAccess { target, key, .. } => {
            fmt_expr(out, target)?;
            write!(out, ".{}", fmt_ident(key.clone()))?;
        }
        ValueExpr::ListAccess { target, index, .. } => {
            fmt_expr(out, target)?;
            out.push('[');
            fmt_expr(out, index)?;
            out.push(']');
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
            for (index, part) in name.iter().enumerate() {
                if index > 0 {
                    out.push('.');
                }
                out.push_str(&fmt_call_segment(part.clone()));
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
        ValueExpr::Exists {
            pattern, negated, ..
        } => {
            if *negated {
                out.push_str("NOT ");
            }
            out.push_str("EXISTS { ");
            fmt_match(out, pattern)?;
            out.push_str(" }");
        }
        ValueExpr::CountSubquery { pattern, .. } => {
            out.push_str("COUNT { ");
            fmt_match(out, pattern)?;
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
