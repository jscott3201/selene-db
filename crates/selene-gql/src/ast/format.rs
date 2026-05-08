//! Read-side AST pretty-printer.

use std::fmt::{self, Write as _};

use selene_core::IStr;

use crate::ast::{
    BinaryOp, EdgeDirection, EdgePattern, GraphPattern, IsCheckKind, LabelExpr, LimitValue,
    MatchClause, MatchMode, NodePattern, NormalForm, NullsPolicy, OrderDirection, OrderTerm,
    PathMode, PathSelector, PatternElement, ProcedureCall, QueryPipeline, ReturnClause, ReturnItem,
    SetOp, Statement, TruthValue, UnaryOp, ValueExpr, WithClause,
};

/// Format a read-side statement as GQL source.
///
/// # Errors
///
/// Returns [`FormatError::Unsupported`] for write-side AST surfaces.
pub fn format_statement(stmt: &Statement) -> Result<String, FormatError> {
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
        Statement::Mutate(_) => return Err(FormatError::Unsupported("mutation pipeline")),
        Statement::Ddl(_) => return Err(FormatError::Unsupported("DDL statement")),
        Statement::Call(_) => return Err(FormatError::Unsupported("top-level CALL")),
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => {
            return Err(FormatError::Unsupported("transaction control"));
        }
    }
    Ok(out)
}

/// Errors raised by the pretty-printer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    /// AST surface is not formattable in M5a.
    #[error("AST surface {0} is not yet formattable; M5a covers read-side only")]
    Unsupported(&'static str),
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
    if clause.path_mode != PathMode::Walk {
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
        match quantifier.max {
            Some(max) if max == quantifier.min => write!(out, "{{{max}}}")?,
            Some(max) => write!(out, "{{{}, {max}}}", quantifier.min)?,
            None => write!(out, "{{{},}}", quantifier.min)?,
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
        LimitValue::Parameter(name, _) => write!(out, "${}", fmt_ident(*name)),
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

fn fmt_expr(out: &mut String, expr: &ValueExpr) -> fmt::Result {
    match expr {
        ValueExpr::Literal(literal) => match literal {
            crate::Literal::Bool(value, _) => out.push_str(if *value { "true" } else { "false" }),
            crate::Literal::Integer(value, _) => write!(out, "{value}")?,
            crate::Literal::Float(value, _) => write!(out, "{value}")?,
            crate::Literal::String(value, _) => write!(out, "'{}'", escape_string(value.as_str()))?,
            crate::Literal::Null(_) => out.push_str("null"),
        },
        ValueExpr::Variable { name, .. } => out.push_str(&fmt_ident(*name)),
        ValueExpr::Parameter { name, .. } => write!(out, "${}", fmt_ident(*name))?,
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
            out.push('(');
            fmt_expr(out, lhs)?;
            write!(out, " {} ", fmt_binary(*op))?;
            fmt_expr(out, rhs)?;
            out.push(')');
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
                out.push_str(&fmt_ident(*part));
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
        ValueExpr::Like {
            operand,
            pattern,
            negated,
            ..
        } => {
            fmt_expr(out, operand)?;
            if *negated {
                out.push_str(" NOT");
            }
            out.push_str(" LIKE ");
            fmt_expr(out, pattern)?;
        }
        ValueExpr::Between {
            operand,
            low,
            high,
            negated,
            ..
        } => {
            fmt_expr(out, operand)?;
            if *negated {
                out.push_str(" NOT");
            }
            out.push_str(" BETWEEN ");
            fmt_expr(out, low)?;
            out.push_str(" AND ");
            fmt_expr(out, high)?;
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
    }
    Ok(())
}

fn fmt_is_check(
    out: &mut String,
    operand: &ValueExpr,
    kind: &IsCheckKind,
    negated: bool,
) -> fmt::Result {
    fmt_expr(out, operand)?;
    out.push_str(" IS");
    if negated {
        out.push_str(" NOT");
    }
    match kind {
        IsCheckKind::Null => out.push_str(" NULL"),
        IsCheckKind::Directed => out.push_str(" DIRECTED"),
        IsCheckKind::Labeled(label) => {
            out.push_str(" LABELED ");
            fmt_label_expr(out, label)?;
        }
        IsCheckKind::TruthValue(value) => {
            out.push(' ');
            out.push_str(match value {
                TruthValue::True => "TRUE",
                TruthValue::False => "FALSE",
                TruthValue::Unknown => "UNKNOWN",
            });
        }
        IsCheckKind::Typed(ty) => write!(out, " TYPED {}", fmt_type(ty))?,
        IsCheckKind::Normalized(form) => {
            out.push(' ');
            out.push_str(match form {
                NormalForm::Nfc => "NFC",
                NormalForm::Nfd => "NFD",
                NormalForm::Nfkc => "NFKC",
                NormalForm::Nfkd => "NFKD",
            });
            out.push_str(" NORMALIZED");
        }
        IsCheckKind::SourceOf(value) => {
            out.push_str(" SOURCE OF ");
            fmt_expr(out, value)?;
        }
        IsCheckKind::DestinationOf(value) => {
            out.push_str(" DESTINATION OF ");
            fmt_expr(out, value)?;
        }
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

fn fmt_set_op(op: SetOp) -> &'static str {
    match op {
        SetOp::Union => "UNION",
        SetOp::UnionAll => "UNION ALL",
        SetOp::Intersect => "INTERSECT",
        SetOp::IntersectAll => "INTERSECT ALL",
        SetOp::Except => "EXCEPT",
        SetOp::ExceptAll => "EXCEPT ALL",
        SetOp::Otherwise => "OTHERWISE",
    }
}

fn fmt_binary(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Power => "^",
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "<>",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "AND",
        BinaryOp::Or => "OR",
        BinaryOp::Xor => "XOR",
        BinaryOp::Concat => "||",
        BinaryOp::Contains => "CONTAINS",
        BinaryOp::StartsWith => "STARTS WITH",
        BinaryOp::EndsWith => "ENDS WITH",
    }
}

fn fmt_path_selector(selector: PathSelector) -> &'static str {
    match selector {
        PathSelector::Any => "ANY",
        PathSelector::All => "ALL",
        PathSelector::AnyShortest => "ANY SHORTEST",
        PathSelector::AllShortest => "ALL SHORTEST",
    }
}

fn fmt_match_mode(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::DifferentEdges => "DIFFERENT EDGES",
        MatchMode::RepeatableElements => "REPEATABLE ELEMENTS",
    }
}

fn fmt_path_mode(mode: PathMode) -> &'static str {
    match mode {
        PathMode::Walk => "WALK",
        PathMode::Trail => "TRAIL",
        PathMode::Acyclic => "ACYCLIC",
        PathMode::Simple => "SIMPLE",
    }
}

fn fmt_type(ty: &crate::GqlType) -> &'static str {
    match ty {
        crate::GqlType::String => "STRING",
        crate::GqlType::Boolean => "BOOLEAN",
        crate::GqlType::Integer => "INTEGER",
        crate::GqlType::Float => "FLOAT",
        crate::GqlType::Int8 => "INT8",
        crate::GqlType::Int16 => "INT16",
        crate::GqlType::Int32 => "INT32",
        crate::GqlType::Int64 => "INT64",
        crate::GqlType::Int128 => "INT128",
        crate::GqlType::Uint8 => "UINT8",
        crate::GqlType::Uint16 => "UINT16",
        crate::GqlType::Uint32 => "UINT32",
        crate::GqlType::Uint64 => "UINT64",
        crate::GqlType::Uint128 => "UINT128",
        crate::GqlType::SmallInt => "SMALLINT",
        crate::GqlType::BigInt => "BIGINT",
        crate::GqlType::Decimal => "DECIMAL",
        crate::GqlType::Float32 => "FLOAT32",
        crate::GqlType::Float64 => "FLOAT64",
        crate::GqlType::Bytes => "BYTES",
        crate::GqlType::Binary => "BYTES",
        crate::GqlType::VarBinary => "BYTES",
        crate::GqlType::ZonedDateTime => "ZONED DATETIME",
        crate::GqlType::LocalDateTime => "LOCAL DATETIME",
        crate::GqlType::Date => "DATE",
        crate::GqlType::ZonedTime => "ZONED TIME",
        crate::GqlType::LocalTime => "LOCAL TIME",
        crate::GqlType::Duration => "DURATION",
        crate::GqlType::List(_) => "LIST<STRING>",
        crate::GqlType::Path => "PATH",
        crate::GqlType::Null => "NULL",
        crate::GqlType::Nothing => "NOTHING",
        crate::GqlType::Record(_)
        | crate::GqlType::GraphRef
        | crate::GqlType::NodeRef
        | crate::GqlType::EdgeRef
        | crate::GqlType::TableRef => "STRING",
    }
}

fn fmt_ident(value: IStr) -> String {
    let value = value.as_str();
    if is_simple_ident(value) && !KEYWORDS.contains(&value.to_ascii_uppercase().as_str()) {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[rustfmt::skip]
const KEYWORDS: &[&str] = &["MATCH", "RETURN", "FILTER", "WHERE", "ORDER", "LIMIT", "OFFSET", "GROUP", "LET", "SET", "DELETE", "INSERT", "REMOVE", "CALL", "YIELD", "AS", "START", "COMMIT", "ROLLBACK", "UNION", "INTERSECT", "EXCEPT", "OTHERWISE", "NEXT"];

fn is_simple_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic()) && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\'', "''")
}
