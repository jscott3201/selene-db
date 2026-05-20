//! Multi-statement parser entry point and statement-boundary scanner.

use crate::{
    ast::{
        DdlStatement, EdgePattern, GraphPattern, IsCheckKind, MatchClause, MutationPipeline,
        MutationStatement, MutationTerminator, NodePattern, PatternElement, ProcedureCall,
        QueryPipeline, ReturnClause, ReturnItem, SetItem, SourceSpan, Statement,
        TypePropertyConstraint, TypePropertyDef, ValueExpr, WithClause,
    },
    error::ParserError,
};

use super::{parse, to_u32};

/// Parse semicolon-separated GQL statements.
///
/// Semicolons inside strings, delimited identifiers, and comments do not split
/// statements. Empty segments are skipped.
///
/// # Errors
///
/// Returns the first [`ParserError`] produced by a non-empty segment, with its
/// source span rebased to the original multi-statement source.
pub fn parse_many(source: &str) -> Result<Vec<Statement>, ParserError> {
    let segments = scan_statement_boundaries(source);
    let mut statements = Vec::with_capacity(segments.len());
    for (start, slice) in segments {
        if slice.trim().is_empty() {
            continue;
        }
        match parse(slice) {
            Ok(mut statement) => {
                rebase_statement_spans(&mut statement, start);
                statements.push(statement);
            }
            Err(mut error) => {
                rebase_parser_error(&mut error, start);
                return Err(error);
            }
        }
    }
    Ok(statements)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanState {
    Normal,
    SingleQuote,
    DoubleQuote,
    Backtick,
    LineComment,
    BlockComment,
}

fn scan_statement_boundaries(source: &str) -> Vec<(usize, &str)> {
    let bytes = source.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut state = ScanState::Normal;
    while index < bytes.len() {
        match state {
            ScanState::Normal => match bytes[index] {
                b';' => {
                    segments.push((start, &source[start..index]));
                    index += 1;
                    start = index;
                }
                b'\'' => {
                    state = ScanState::SingleQuote;
                    index += 1;
                }
                b'"' => {
                    state = ScanState::DoubleQuote;
                    index += 1;
                }
                b'`' => {
                    state = ScanState::Backtick;
                    index += 1;
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    state = ScanState::LineComment;
                    index += 2;
                }
                b'-' if bytes.get(index + 1) == Some(&b'-') => {
                    state = ScanState::LineComment;
                    index += 2;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    state = ScanState::BlockComment;
                    index += 2;
                }
                _ => index += 1,
            },
            ScanState::SingleQuote => match bytes[index] {
                b'\\' => index = (index + 2).min(bytes.len()),
                b'\'' if bytes.get(index + 1) == Some(&b'\'') => index += 2,
                b'\'' => {
                    state = ScanState::Normal;
                    index += 1;
                }
                _ => index += 1,
            },
            ScanState::DoubleQuote => match bytes[index] {
                b'"' if bytes.get(index + 1) == Some(&b'"') => index += 2,
                b'"' => {
                    state = ScanState::Normal;
                    index += 1;
                }
                _ => index += 1,
            },
            ScanState::Backtick => {
                if bytes[index] == b'`' {
                    state = ScanState::Normal;
                }
                index += 1;
            }
            ScanState::LineComment => {
                if bytes[index] == b'\n' {
                    state = ScanState::Normal;
                }
                index += 1;
            }
            ScanState::BlockComment => {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = ScanState::Normal;
                    index += 2;
                } else {
                    index += 1;
                }
            }
        }
    }
    segments.push((start, &source[start..]));
    segments
}

fn rebase_parser_error(error: &mut ParserError, offset: usize) {
    match error {
        ParserError::SyntaxError { span, .. }
        | ParserError::UnsupportedFeature { span, .. }
        | ParserError::InternerBudgetExceeded { span, .. }
        | ParserError::NestingLimitExceeded { span, .. }
        | ParserError::NotImplemented { span, .. } => rebase_span(span, offset),
    }
}

fn rebase_statement_spans(statement: &mut Statement, offset: usize) {
    match statement {
        Statement::Query(pipeline) => rebase_query_pipeline(pipeline, offset),
        Statement::Composite { first, rest, span } => {
            rebase_span(span, offset);
            rebase_query_pipeline(first, offset);
            for (_, pipeline) in rest {
                rebase_query_pipeline(pipeline, offset);
            }
        }
        Statement::Chained { blocks, span } => {
            rebase_span(span, offset);
            for pipeline in blocks {
                rebase_query_pipeline(pipeline, offset);
            }
        }
        Statement::Mutate(pipeline) => rebase_mutation_pipeline(pipeline, offset),
        Statement::Ddl(statement) => rebase_ddl(statement, offset),
        Statement::Call(call) => rebase_call(call, offset),
        Statement::Explain { inner, span } => {
            rebase_span(span, offset);
            rebase_statement_spans(inner, offset);
        }
        Statement::StartTransaction { span }
        | Statement::Commit { span }
        | Statement::Rollback { span } => rebase_span(span, offset),
    }
}

fn rebase_query_pipeline(pipeline: &mut QueryPipeline, offset: usize) {
    rebase_span(&mut pipeline.span, offset);
    for statement in &mut pipeline.statements {
        match statement {
            crate::PipelineStatement::Match(value) => rebase_match(value, offset),
            crate::PipelineStatement::Filter(value) => rebase_value(value, offset),
            crate::PipelineStatement::Let(values) => {
                for value in values {
                    rebase_span(&mut value.span, offset);
                    rebase_value(&mut value.value, offset);
                }
            }
            crate::PipelineStatement::Unwind(value) => {
                rebase_span(&mut value.span, offset);
                rebase_value(&mut value.source, offset);
            }
            crate::PipelineStatement::Sorting(values) => {
                for value in values {
                    rebase_span(&mut value.span, offset);
                    rebase_value(&mut value.expr, offset);
                }
            }
            crate::PipelineStatement::Limit(value) | crate::PipelineStatement::Offset(value) => {
                rebase_limit(value, offset);
            }
            crate::PipelineStatement::Return(value) => rebase_return(value, offset),
            crate::PipelineStatement::With(value) => rebase_with(value, offset),
            crate::PipelineStatement::Call(value) => rebase_call(value, offset),
        }
    }
}

fn rebase_limit(value: &mut crate::LimitValue, offset: usize) {
    match value {
        crate::LimitValue::Count(_, span) | crate::LimitValue::Parameter(_, span) => {
            rebase_span(span, offset);
        }
    }
}

fn rebase_return(clause: &mut ReturnClause, offset: usize) {
    rebase_span(&mut clause.span, offset);
    for item in &mut clause.items {
        rebase_return_item(item, offset);
    }
    if let Some(group_by) = &mut clause.group_by {
        for item in group_by {
            rebase_value(item, offset);
        }
    }
    if let Some(having) = &mut clause.having {
        rebase_value(having, offset);
    }
}

fn rebase_return_item(item: &mut ReturnItem, offset: usize) {
    rebase_span(&mut item.span, offset);
    rebase_value(&mut item.expr, offset);
}

fn rebase_with(clause: &mut WithClause, offset: usize) {
    rebase_span(&mut clause.span, offset);
    for item in &mut clause.items {
        rebase_return_item(item, offset);
    }
    if let Some(group_by) = &mut clause.group_by {
        for item in group_by {
            rebase_value(item, offset);
        }
    }
    if let Some(having) = &mut clause.having {
        rebase_value(having, offset);
    }
    if let Some(where_clause) = &mut clause.where_clause {
        rebase_value(where_clause, offset);
    }
}

fn rebase_match(clause: &mut MatchClause, offset: usize) {
    rebase_span(&mut clause.span, offset);
    for pattern in &mut clause.patterns {
        rebase_graph_pattern(pattern, offset);
    }
    if let Some(where_clause) = &mut clause.where_clause {
        rebase_value(where_clause, offset);
    }
}

fn rebase_graph_pattern(pattern: &mut GraphPattern, offset: usize) {
    rebase_span(&mut pattern.span, offset);
    for element in &mut pattern.elements {
        match element {
            PatternElement::Node(node) => rebase_node_pattern(node, offset),
            PatternElement::Edge(edge) => rebase_edge_pattern(edge, offset),
        }
    }
}

fn rebase_node_pattern(pattern: &mut NodePattern, offset: usize) {
    rebase_span(&mut pattern.span, offset);
    for (_, value) in &mut pattern.properties {
        rebase_value(value, offset);
    }
    if let Some(value) = &mut pattern.inline_where {
        rebase_value(value, offset);
    }
}

fn rebase_edge_pattern(pattern: &mut EdgePattern, offset: usize) {
    rebase_span(&mut pattern.span, offset);
    for (_, value) in &mut pattern.properties {
        rebase_value(value, offset);
    }
    if let Some(value) = &mut pattern.inline_where {
        rebase_value(value, offset);
    }
}

fn rebase_value(value: &mut ValueExpr, offset: usize) {
    match value {
        ValueExpr::Literal(literal) => match literal {
            crate::Literal::Bool(_, span)
            | crate::Literal::Integer(_, span)
            | crate::Literal::Float(_, span)
            | crate::Literal::String(_, span)
            | crate::Literal::Null(span) => rebase_span(span, offset),
        },
        ValueExpr::Variable { span, .. } | ValueExpr::Parameter { span, .. } => {
            rebase_span(span, offset);
        }
        ValueExpr::PropertyAccess { target, span, .. } => {
            rebase_span(span, offset);
            rebase_value(target, offset);
        }
        ValueExpr::ListAccess {
            target,
            index,
            span,
        } => {
            rebase_span(span, offset);
            rebase_value(target, offset);
            rebase_value(index, offset);
        }
        ValueExpr::ListLiteral { items, span } => {
            rebase_span(span, offset);
            for item in items {
                rebase_value(item, offset);
            }
        }
        ValueExpr::RecordLiteral { fields, span } => {
            rebase_span(span, offset);
            for (_, value) in fields {
                rebase_value(value, offset);
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, span, .. } => {
            rebase_span(span, offset);
            rebase_value(lhs, offset);
            rebase_value(rhs, offset);
        }
        ValueExpr::UnaryOp { operand, span, .. } => {
            rebase_span(span, offset);
            rebase_value(operand, offset);
        }
        ValueExpr::FunctionCall { args, span, .. } => {
            rebase_span(span, offset);
            for arg in args {
                rebase_value(arg, offset);
            }
        }
        ValueExpr::IsCheck {
            operand,
            kind,
            span,
            ..
        } => {
            rebase_span(span, offset);
            rebase_value(operand, offset);
            rebase_is_check(kind, offset);
        }
        ValueExpr::InList {
            operand,
            list,
            span,
            ..
        } => {
            rebase_span(span, offset);
            rebase_value(operand, offset);
            for item in list {
                rebase_value(item, offset);
            }
        }
        ValueExpr::Like {
            operand,
            pattern,
            span,
            ..
        } => {
            rebase_span(span, offset);
            rebase_value(operand, offset);
            rebase_value(pattern, offset);
        }
        ValueExpr::Between {
            operand,
            low,
            high,
            span,
            ..
        } => {
            rebase_span(span, offset);
            rebase_value(operand, offset);
            rebase_value(low, offset);
            rebase_value(high, offset);
        }
        ValueExpr::AllDifferent { items, span } | ValueExpr::Same { items, span } => {
            rebase_span(span, offset);
            for item in items {
                rebase_value(item, offset);
            }
        }
        ValueExpr::PropertyExists { target, span, .. } => {
            rebase_span(span, offset);
            rebase_value(target, offset);
        }
        ValueExpr::Case {
            branches,
            else_branch,
            span,
        } => {
            rebase_span(span, offset);
            for (condition, result) in branches {
                rebase_value(condition, offset);
                rebase_value(result, offset);
            }
            if let Some(value) = else_branch {
                rebase_value(value, offset);
            }
        }
        ValueExpr::Exists { pattern, span, .. } | ValueExpr::CountSubquery { pattern, span } => {
            rebase_span(span, offset);
            rebase_match(pattern, offset);
        }
    }
}

fn rebase_is_check(kind: &mut IsCheckKind, offset: usize) {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
            rebase_value(value, offset);
        }
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::Normalized(_) => {}
    }
}

fn rebase_mutation_pipeline(pipeline: &mut MutationPipeline, offset: usize) {
    rebase_span(&mut pipeline.span, offset);
    for statement in &mut pipeline.statements {
        match statement {
            MutationStatement::Match(value) => rebase_match(value, offset),
            MutationStatement::Filter(value) => rebase_value(value, offset),
            MutationStatement::Insert(value) => {
                rebase_span(&mut value.span, offset);
                for pattern in &mut value.patterns {
                    rebase_graph_pattern(pattern, offset);
                }
            }
            MutationStatement::Set(values) => {
                for value in values {
                    match value {
                        SetItem::Property { value, span, .. } => {
                            rebase_span(span, offset);
                            rebase_value(value, offset);
                        }
                        SetItem::PropertyMerge {
                            properties, span, ..
                        } => {
                            rebase_span(span, offset);
                            for (_, value) in properties {
                                rebase_value(value, offset);
                            }
                        }
                        SetItem::Label { span, .. } => rebase_span(span, offset),
                    }
                }
            }
            MutationStatement::Remove(values) => {
                for value in values {
                    match value {
                        crate::RemoveItem::Property { span, .. }
                        | crate::RemoveItem::Label { span, .. } => rebase_span(span, offset),
                    }
                }
            }
            MutationStatement::Delete(value) => rebase_span(&mut value.span, offset),
        }
    }
    if let Some(terminator) = &mut pipeline.terminator {
        match terminator {
            MutationTerminator::Return(value) => rebase_return(value, offset),
            MutationTerminator::Finish(span) => rebase_span(span, offset),
        }
    }
}

fn rebase_ddl(statement: &mut DdlStatement, offset: usize) {
    match statement {
        DdlStatement::CreateGraph { span, .. }
        | DdlStatement::DropGraph { span, .. }
        | DdlStatement::DropNodeType { span, .. }
        | DdlStatement::DropEdgeType { span, .. } => rebase_span(span, offset),
        DdlStatement::CreateNodeType {
            properties, span, ..
        }
        | DdlStatement::CreateEdgeType {
            properties, span, ..
        } => {
            rebase_span(span, offset);
            for property in properties {
                rebase_property_def(property, offset);
            }
        }
        DdlStatement::ShowNodeTypes(span)
        | DdlStatement::ShowEdgeTypes(span)
        | DdlStatement::ShowIndexes(span)
        | DdlStatement::ShowProcedures(span) => {
            rebase_span(span, offset);
        }
    }
}

fn rebase_property_def(property: &mut TypePropertyDef, offset: usize) {
    rebase_span(&mut property.span, offset);
    for constraint in &mut property.constraints {
        match constraint {
            TypePropertyConstraint::NotNull(span)
            | TypePropertyConstraint::Immutable(span)
            | TypePropertyConstraint::Unique(span)
            | TypePropertyConstraint::Indexed(span)
            | TypePropertyConstraint::Searchable(span)
            | TypePropertyConstraint::Dictionary(span)
            | TypePropertyConstraint::Fill(_, span)
            | TypePropertyConstraint::Interval(_, span)
            | TypePropertyConstraint::Encoding(_, span) => rebase_span(span, offset),
            TypePropertyConstraint::Default(value, span) => {
                rebase_span(span, offset);
                rebase_value(value, offset);
            }
        }
    }
}

fn rebase_call(call: &mut ProcedureCall, offset: usize) {
    rebase_span(&mut call.span, offset);
    for arg in &mut call.args {
        rebase_value(arg, offset);
    }
    for item in &mut call.yield_items {
        rebase_span(&mut item.span, offset);
    }
}

fn rebase_span(span: &mut SourceSpan, offset: usize) {
    span.byte_offset = span.byte_offset.saturating_add(to_u32(offset));
}
