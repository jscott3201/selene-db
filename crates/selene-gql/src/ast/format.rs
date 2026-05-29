//! Read-side AST pretty-printer.

mod cast;
mod is_check;
mod keywords;
mod preflight;
mod trim;
mod type_name;

use std::fmt::{self, Write as _};

use selene_core::IStr;

use super::format_ident::{escape_string, fmt_call_segment, fmt_ident};
use crate::ast::{
    EdgeDirection, EdgePattern, GqlType, GraphPattern, InlineProcedureCall, LabelExpr, LimitValue,
    MatchClause, NodePattern, NullsPolicy, OrderDirection, OrderTerm, PathMode, PatternElement,
    ProcedureCall, Quantifier, QueryPipeline, ReturnClause, ReturnItem, Statement, UnaryOp,
    ValueExpr, WithClause,
};

use is_check::{fmt_is_check, fmt_normal_form};
use keywords::{fmt_binary, fmt_match_mode, fmt_path_mode, fmt_path_selector, fmt_set_op};
pub use preflight::validate_formattable;
use type_name::fmt_type;

/// Format a read-side statement as GQL source.
///
/// # Errors
///
/// Returns [`FormatError::Unsupported`] for write-side AST surfaces.
pub fn format_read_statement(stmt: &Statement) -> Result<String, FormatError> {
    validate_formattable(stmt)?;

    let mut out = String::new();
    match stmt {
        Statement::Query(pipeline) => fmt_pipeline(&mut out, pipeline)?,
        Statement::Composite { first, rest, .. } => {
            fmt_pipeline(&mut out, first)?;
            for (op, pipeline) in rest {
                write!(out, " {} ", fmt_set_op(*op))?;
                fmt_pipeline(&mut out, pipeline)?;
            }
        }
        Statement::Chained { blocks, .. } => {
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    out.push_str(" NEXT ");
                }
                fmt_pipeline(&mut out, block)?;
            }
        }
        Statement::Mutate(_) => {
            return Err(FormatError::Unsupported {
                variant: "MutateStatement",
            });
        }
        Statement::Ddl(_) => {
            return Err(FormatError::Unsupported {
                variant: "DdlStatement",
            });
        }
        Statement::Call(_) => {
            return Err(FormatError::Unsupported {
                variant: "ProcedureCall",
            });
        }
        Statement::Explain { .. } => {
            return Err(FormatError::Unsupported {
                variant: "ExplainStatement",
            });
        }
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => {
            return Err(FormatError::Unsupported {
                variant: "TransactionControl",
            });
        }
        Statement::SessionSetValue { .. }
        | Statement::SessionSetTimeZone { .. }
        | Statement::SessionReset { .. }
        | Statement::SessionClose { .. } => {
            return Err(FormatError::Unsupported {
                variant: "SessionControl",
            });
        }
    }
    Ok(out)
}

/// Format one procedure call as canonical GQL source.
///
/// # Errors
///
/// Returns [`FormatError::Unsupported`] when the call contains an AST-only
/// expression surface that the parser cannot read back from formatted source.
pub fn format_procedure_call(call: &ProcedureCall) -> Result<String, FormatError> {
    preflight::validate_procedure_call(call)?;

    let mut out = String::new();
    fmt_call(&mut out, call)?;
    Ok(out)
}

/// Reserved mutation-formatter entry point.
///
/// # Errors
///
/// Always returns [`FormatError::Unsupported`] until mutation formatting is
/// implemented.
pub fn format_mutate_statement(_stmt: &Statement) -> Result<String, FormatError> {
    Err(FormatError::Unsupported {
        variant: "MutateStatement",
    })
}

/// Errors raised by the pretty-printer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    /// AST surface is not formattable by this read-side pretty-printer.
    #[error("AST surface {variant} is not yet formattable by the read-side pretty-printer")]
    Unsupported {
        /// Stable AST variant name.
        variant: &'static str,
    },
    /// Formatting failed.
    #[error("formatting failed")]
    Fmt(#[from] fmt::Error),
}

fn fmt_pipeline(out: &mut String, pipeline: &QueryPipeline) -> fmt::Result {
    for (index, statement) in pipeline.statements.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        match statement {
            crate::PipelineStatement::Match(value) => fmt_match(out, value)?,
            crate::PipelineStatement::Filter(value) => {
                out.push_str("FILTER ");
                fmt_expr(out, value)?;
            }
            crate::PipelineStatement::Let(values) => {
                out.push_str("LET ");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    write!(out, "{} = ", fmt_ident(value.alias))?;
                    fmt_expr(out, &value.value)?;
                }
            }
            crate::PipelineStatement::Unwind(value) => {
                out.push_str("UNWIND ");
                fmt_expr(out, &value.source)?;
                write!(out, " AS {}", fmt_ident(value.alias))?;
            }
            crate::PipelineStatement::Sorting(values) => fmt_order(out, values)?,
            crate::PipelineStatement::Limit(value) => {
                out.push_str("LIMIT ");
                fmt_limit(out, value)?;
            }
            crate::PipelineStatement::Offset(value) => {
                out.push_str("OFFSET ");
                fmt_limit(out, value)?;
            }
            crate::PipelineStatement::Return(value) => fmt_return(out, value)?,
            crate::PipelineStatement::With(value) => fmt_with(out, value)?,
            crate::PipelineStatement::Call(value) => fmt_call(out, value)?,
            crate::PipelineStatement::CallSubquery(value) => fmt_inline_call(out, value)?,
        }
    }
    Ok(())
}

fn fmt_match(out: &mut String, clause: &MatchClause) -> fmt::Result {
    if clause.optional {
        out.push_str("OPTIONAL ");
    }
    out.push_str("MATCH");
    if let Some(selector) = clause.selector {
        write!(out, " {}", fmt_path_selector(selector))?;
    }
    if let Some(mode) = clause.match_mode {
        write!(out, " {}", fmt_match_mode(mode))?;
    }
    if clause.path_mode != PathMode::Walk
        || (clause.path_mode == PathMode::Walk && clause.path_mode_explicit)
    {
        write!(out, " {}", fmt_path_mode(clause.path_mode))?;
    }
    out.push(' ');
    for (index, pattern) in clause.patterns.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        fmt_graph_pattern(out, pattern)?;
    }
    if let Some(where_clause) = &clause.where_clause {
        out.push_str(" WHERE ");
        fmt_expr(out, where_clause)?;
    }
    Ok(())
}

fn fmt_graph_pattern(out: &mut String, pattern: &GraphPattern) -> fmt::Result {
    if let Some(binding) = pattern.path_binding {
        write!(out, "{} = ", fmt_ident(binding))?;
    }
    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => fmt_node_pattern(out, node)?,
            PatternElement::Edge(edge) => fmt_edge_pattern(out, edge)?,
        }
    }
    Ok(())
}

fn fmt_node_pattern(out: &mut String, node: &NodePattern) -> fmt::Result {
    out.push('(');
    if let Some(binding) = node.binding {
        out.push_str(&fmt_ident(binding));
    }
    if let Some(label) = &node.label_expr {
        fmt_label_expr(out, label)?;
    }
    fmt_properties(out, &node.properties)?;
    if let Some(where_clause) = &node.inline_where {
        out.push_str(" WHERE ");
        fmt_expr(out, where_clause)?;
    }
    out.push(')');
    Ok(())
}

fn fmt_edge_pattern(out: &mut String, edge: &EdgePattern) -> fmt::Result {
    match edge.direction {
        EdgeDirection::Right | EdgeDirection::Undirected => out.push_str("-["),
        EdgeDirection::Left => out.push_str("<-["),
    }
    if let Some(binding) = edge.binding {
        out.push_str(&fmt_ident(binding));
    }
    if let Some(label) = &edge.label_expr {
        fmt_label_expr(out, label)?;
    }
    if let Some(quantifier) = edge.quantifier {
        match quantifier {
            Quantifier::GraphPattern {
                min,
                max: Some(max),
            } if max == min => write!(out, "{{{max}}}")?,
            Quantifier::GraphPattern {
                min,
                max: Some(max),
            } => write!(out, "{{{min},{max}}}")?,
            Quantifier::GraphPattern { min, max: None } => write!(out, "{{{min},}}")?,
            Quantifier::Questioned => out.push('?'),
        }
    }
    fmt_properties(out, &edge.properties)?;
    if let Some(where_clause) = &edge.inline_where {
        out.push_str(" WHERE ");
        fmt_expr(out, where_clause)?;
    }
    match edge.direction {
        EdgeDirection::Right => out.push_str("]->"),
        EdgeDirection::Left | EdgeDirection::Undirected => out.push_str("]-"),
    }
    Ok(())
}

fn fmt_properties(out: &mut String, properties: &[(IStr, ValueExpr)]) -> fmt::Result {
    if properties.is_empty() {
        return Ok(());
    }
    out.push_str(" {");
    for (index, (key, value)) in properties.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write!(out, "{}: ", fmt_ident(*key))?;
        fmt_expr(out, value)?;
    }
    out.push('}');
    Ok(())
}

fn fmt_label_expr(out: &mut String, label: &LabelExpr) -> fmt::Result {
    match label {
        LabelExpr::Single(label) => write!(out, ":{}", fmt_ident(*label)),
        LabelExpr::Conjunction(parts) => fmt_label_parts(out, parts, "&"),
        LabelExpr::Disjunction(parts) => fmt_label_parts(out, parts, "|"),
        LabelExpr::Negation(inner) => {
            out.push_str(":!");
            fmt_label_expr_body(out, inner)
        }
        LabelExpr::Wildcard => {
            out.push_str(":%");
            Ok(())
        }
    }
}

fn fmt_label_expr_body(out: &mut String, label: &LabelExpr) -> fmt::Result {
    match label {
        LabelExpr::Single(label) => write!(out, "{}", fmt_ident(*label)),
        _ => fmt_label_expr(out, label),
    }
}

fn fmt_label_parts(out: &mut String, parts: &[LabelExpr], sep: &str) -> fmt::Result {
    out.push(':');
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            out.push_str(sep);
        }
        fmt_label_expr_body(out, part)?;
    }
    Ok(())
}

fn fmt_return(out: &mut String, clause: &ReturnClause) -> fmt::Result {
    out.push_str("RETURN ");
    if clause.distinct {
        out.push_str("DISTINCT ");
    }
    if clause.star {
        out.push('*');
    } else {
        fmt_return_items(out, &clause.items)?;
    }
    fmt_group_having(out, clause.group_by.as_deref(), clause.having.as_ref())
}

fn fmt_with(out: &mut String, clause: &WithClause) -> fmt::Result {
    out.push_str("WITH ");
    if clause.distinct {
        out.push_str("DISTINCT ");
    }
    fmt_return_items(out, &clause.items)?;
    fmt_group_having(out, clause.group_by.as_deref(), clause.having.as_ref())?;
    if let Some(where_clause) = &clause.where_clause {
        out.push_str(" WHERE ");
        fmt_expr(out, where_clause)?;
    }
    Ok(())
}

fn fmt_return_items(out: &mut String, items: &[ReturnItem]) -> fmt::Result {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        fmt_expr(out, &item.expr)?;
        if let Some(alias) = item.alias {
            write!(out, " AS {}", fmt_ident(alias))?;
        }
    }
    Ok(())
}

fn fmt_group_having(
    out: &mut String,
    group_by: Option<&[ValueExpr]>,
    having: Option<&ValueExpr>,
) -> fmt::Result {
    if let Some(group_by) = group_by {
        out.push_str(" GROUP BY ");
        for (index, item) in group_by.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            fmt_expr(out, item)?;
        }
    }
    if let Some(having) = having {
        out.push_str(" HAVING ");
        fmt_expr(out, having)?;
    }
    Ok(())
}

fn fmt_order(out: &mut String, terms: &[OrderTerm]) -> fmt::Result {
    out.push_str("ORDER BY ");
    for (index, term) in terms.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        fmt_expr(out, &term.expr)?;
        if term.direction == OrderDirection::Desc {
            out.push_str(" DESC");
        }
        if let Some(nulls) = term.nulls {
            out.push_str(match nulls {
                NullsPolicy::NullsFirst => " NULLS FIRST",
                NullsPolicy::NullsLast => " NULLS LAST",
            });
        }
    }
    Ok(())
}

fn fmt_limit(out: &mut String, value: &LimitValue) -> fmt::Result {
    match value {
        LimitValue::Count(value, _) => write!(out, "{value}"),
        LimitValue::Parameter {
            name,
            declared_type,
            ..
        } => fmt_parameter(out, *name, declared_type.as_ref()),
    }
}

fn fmt_call(out: &mut String, call: &ProcedureCall) -> fmt::Result {
    out.push_str("CALL ");
    for (index, part) in call.name.iter().enumerate() {
        if index > 0 {
            out.push('.');
        }
        out.push_str(&fmt_ident(*part));
    }
    out.push('(');
    for (index, arg) in call.args.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        fmt_expr(out, arg)?;
    }
    out.push(')');
    if !call.yield_items.is_empty() {
        out.push_str(" YIELD ");
        for (index, item) in call.yield_items.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            match item.column {
                crate::YieldColumn::Star => out.push('*'),
                crate::YieldColumn::Named(name) => out.push_str(&fmt_ident(name)),
            }
            if let Some(alias) = item.alias {
                write!(out, " AS {}", fmt_ident(alias))?;
            }
        }
    }
    Ok(())
}

fn fmt_inline_call(out: &mut String, call: &InlineProcedureCall) -> fmt::Result {
    out.push_str("CALL ");
    if let Some(scope) = &call.variable_scope {
        out.push('(');
        for (index, name) in scope.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&fmt_ident(*name));
        }
        out.push_str(") ");
    }
    out.push_str("{ ");
    fmt_pipeline(out, &call.body)?;
    out.push_str(" }");
    if !call.yield_items.is_empty() {
        out.push_str(" YIELD ");
        fmt_yield_items(out, &call.yield_items)?;
    }
    if call.in_transactions {
        out.push_str(" IN TRANSACTIONS");
    }
    Ok(())
}

fn fmt_yield_items(out: &mut String, items: &[crate::YieldItem]) -> fmt::Result {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        match item.column {
            crate::YieldColumn::Star => out.push('*'),
            crate::YieldColumn::Named(name) => out.push_str(&fmt_ident(name)),
        }
        if let Some(alias) = item.alias {
            write!(out, " AS {}", fmt_ident(alias))?;
        }
    }
    Ok(())
}

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
        ValueExpr::Variable { name, .. } => out.push_str(&fmt_ident(*name)),
        ValueExpr::Parameter {
            name,
            declared_type,
            ..
        } => fmt_parameter(out, *name, declared_type.as_ref())?,
        ValueExpr::PropertyAccess { target, key, .. } => {
            fmt_expr(out, target)?;
            write!(out, ".{}", fmt_ident(*key))?;
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
                write!(out, "{}: ", fmt_ident(*key))?;
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
                out.push_str(&fmt_call_segment(*part));
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

fn fmt_parameter(out: &mut String, name: IStr, declared_type: Option<&GqlType>) -> fmt::Result {
    write!(out, "${}", fmt_ident(name))?;
    if let Some(declared_type) = declared_type {
        write!(out, " :: {}", fmt_type(declared_type))?;
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
