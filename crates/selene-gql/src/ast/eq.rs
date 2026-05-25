//! Span-erasing AST equality helpers.

use crate::ast::{
    DdlStatement, EdgePattern, GraphPattern, InlineProcedureCall, IsCheckKind, MatchClause,
    MutationPipeline, MutationStatement, MutationTerminator, NodePattern, PatternElement,
    ProcedureCall, QueryPipeline, ReturnClause, ReturnItem, SetItem, Statement,
    TypePropertyConstraint, TypePropertyDef, ValueExpr, WithClause,
};

use super::SourceSpan;

/// Return true when two statements are structurally equal after removing spans.
#[must_use]
pub fn structurally_eq(a: &Statement, b: &Statement) -> bool {
    let mut left = a.clone();
    let mut right = b.clone();
    scrub_statement(&mut left);
    scrub_statement(&mut right);
    left == right
}

fn scrub_statement(statement: &mut Statement) {
    match statement {
        Statement::Query(pipeline) => scrub_query_pipeline(pipeline),
        Statement::Composite { first, rest, span } => {
            *span = SourceSpan::default();
            scrub_query_pipeline(first);
            for (_, pipeline) in rest {
                scrub_query_pipeline(pipeline);
            }
        }
        Statement::Chained { blocks, span } => {
            *span = SourceSpan::default();
            for pipeline in blocks {
                scrub_query_pipeline(pipeline);
            }
        }
        Statement::Mutate(pipeline) => scrub_mutation_pipeline(pipeline),
        Statement::Ddl(statement) => scrub_ddl(statement),
        Statement::Call(call) => scrub_call(call),
        Statement::Explain { inner, span } => {
            *span = SourceSpan::default();
            scrub_statement(inner);
        }
        Statement::StartTransaction { span }
        | Statement::Commit { span }
        | Statement::Rollback { span } => *span = SourceSpan::default(),
    }
}

fn scrub_query_pipeline(pipeline: &mut QueryPipeline) {
    pipeline.span = SourceSpan::default();
    for statement in &mut pipeline.statements {
        match statement {
            crate::PipelineStatement::Match(value) => scrub_match(value),
            crate::PipelineStatement::Filter(value) => scrub_value(value),
            crate::PipelineStatement::Let(values) => {
                for value in values {
                    value.span = SourceSpan::default();
                    scrub_value(&mut value.value);
                }
            }
            crate::PipelineStatement::Unwind(value) => {
                value.span = SourceSpan::default();
                scrub_value(&mut value.source);
            }
            crate::PipelineStatement::Sorting(values) => {
                for value in values {
                    value.span = SourceSpan::default();
                    scrub_value(&mut value.expr);
                }
            }
            crate::PipelineStatement::Limit(value) | crate::PipelineStatement::Offset(value) => {
                match value {
                    crate::LimitValue::Count(_, span) | crate::LimitValue::Parameter(_, span) => {
                        *span = SourceSpan::default();
                    }
                }
            }
            crate::PipelineStatement::Return(value) => scrub_return(value),
            crate::PipelineStatement::With(value) => scrub_with(value),
            crate::PipelineStatement::Call(value) => scrub_call(value),
            crate::PipelineStatement::CallSubquery(value) => scrub_inline_call(value),
        }
    }
}

fn scrub_return(clause: &mut ReturnClause) {
    clause.span = SourceSpan::default();
    for item in &mut clause.items {
        scrub_return_item(item);
    }
    if let Some(group_by) = &mut clause.group_by {
        for item in group_by {
            scrub_value(item);
        }
    }
    if let Some(having) = &mut clause.having {
        scrub_value(having);
    }
}

fn scrub_return_item(item: &mut ReturnItem) {
    item.span = SourceSpan::default();
    scrub_value(&mut item.expr);
}

fn scrub_with(clause: &mut WithClause) {
    clause.span = SourceSpan::default();
    for item in &mut clause.items {
        scrub_return_item(item);
    }
    if let Some(group_by) = &mut clause.group_by {
        for item in group_by {
            scrub_value(item);
        }
    }
    if let Some(having) = &mut clause.having {
        scrub_value(having);
    }
    if let Some(where_clause) = &mut clause.where_clause {
        scrub_value(where_clause);
    }
}

fn scrub_match(clause: &mut MatchClause) {
    clause.span = SourceSpan::default();
    for pattern in &mut clause.patterns {
        scrub_graph_pattern(pattern);
    }
    if let Some(where_clause) = &mut clause.where_clause {
        scrub_value(where_clause);
    }
}

fn scrub_graph_pattern(pattern: &mut GraphPattern) {
    pattern.span = SourceSpan::default();
    for element in &mut pattern.elements {
        match element {
            PatternElement::Node(node) => scrub_node_pattern(node),
            PatternElement::Edge(edge) => scrub_edge_pattern(edge),
        }
    }
}

fn scrub_node_pattern(pattern: &mut NodePattern) {
    pattern.span = SourceSpan::default();
    for (_, value) in &mut pattern.properties {
        scrub_value(value);
    }
    if let Some(value) = &mut pattern.inline_where {
        scrub_value(value);
    }
}

fn scrub_edge_pattern(pattern: &mut EdgePattern) {
    pattern.span = SourceSpan::default();
    for (_, value) in &mut pattern.properties {
        scrub_value(value);
    }
    if let Some(value) = &mut pattern.inline_where {
        scrub_value(value);
    }
}

fn scrub_value(value: &mut ValueExpr) {
    match value {
        ValueExpr::Literal(literal) => match literal {
            crate::Literal::Bool(_, span)
            | crate::Literal::Integer(_, span)
            | crate::Literal::Float(_, span)
            | crate::Literal::String(_, span)
            | crate::Literal::Uuid(_, span)
            | crate::Literal::Null(span) => *span = SourceSpan::default(),
        },
        ValueExpr::Variable { span, .. } | ValueExpr::Parameter { span, .. } => {
            *span = SourceSpan::default();
        }
        ValueExpr::PropertyAccess { target, span, .. } => {
            *span = SourceSpan::default();
            scrub_value(target);
        }
        ValueExpr::ListAccess {
            target,
            index,
            span,
        } => {
            *span = SourceSpan::default();
            scrub_value(target);
            scrub_value(index);
        }
        ValueExpr::ListLiteral { items, span } => {
            *span = SourceSpan::default();
            for item in items {
                scrub_value(item);
            }
        }
        ValueExpr::RecordLiteral { fields, span } => {
            *span = SourceSpan::default();
            for (_, value) in fields {
                scrub_value(value);
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, span, .. } => {
            *span = SourceSpan::default();
            scrub_value(lhs);
            scrub_value(rhs);
        }
        ValueExpr::UnaryOp { operand, span, .. } => {
            *span = SourceSpan::default();
            scrub_value(operand);
        }
        ValueExpr::FunctionCall { args, span, .. } => {
            *span = SourceSpan::default();
            for arg in args {
                scrub_value(arg);
            }
        }
        ValueExpr::Normalize { source, span, .. } => {
            *span = SourceSpan::default();
            scrub_value(source);
        }
        ValueExpr::Trim {
            character,
            source,
            span,
            ..
        } => {
            *span = SourceSpan::default();
            if let Some(character) = character {
                scrub_value(character);
            }
            scrub_value(source);
        }
        ValueExpr::IsCheck {
            operand,
            kind,
            span,
            ..
        } => {
            *span = SourceSpan::default();
            scrub_value(operand);
            scrub_is_check(kind);
        }
        ValueExpr::InList {
            operand,
            list,
            span,
            ..
        } => {
            *span = SourceSpan::default();
            scrub_value(operand);
            for item in list {
                scrub_value(item);
            }
        }
        ValueExpr::Like {
            operand,
            pattern,
            span,
            ..
        } => {
            *span = SourceSpan::default();
            scrub_value(operand);
            scrub_value(pattern);
        }
        ValueExpr::Between {
            operand,
            low,
            high,
            span,
            ..
        } => {
            *span = SourceSpan::default();
            scrub_value(operand);
            scrub_value(low);
            scrub_value(high);
        }
        ValueExpr::AllDifferent { items, span } | ValueExpr::Same { items, span } => {
            *span = SourceSpan::default();
            for item in items {
                scrub_value(item);
            }
        }
        ValueExpr::PropertyExists { target, span, .. } => {
            *span = SourceSpan::default();
            scrub_value(target);
        }
        ValueExpr::Case {
            branches,
            else_branch,
            span,
        } => {
            *span = SourceSpan::default();
            for (condition, result) in branches {
                scrub_value(condition);
                scrub_value(result);
            }
            if let Some(value) = else_branch {
                scrub_value(value);
            }
        }
        ValueExpr::Exists { pattern, span, .. } | ValueExpr::CountSubquery { pattern, span } => {
            *span = SourceSpan::default();
            scrub_match(pattern);
        }
        ValueExpr::ValueSubquery { body, span } => {
            *span = SourceSpan::default();
            scrub_query_pipeline(body);
        }
        ValueExpr::Cast { value, span, .. } => {
            *span = SourceSpan::default();
            scrub_value(value);
        }
    }
}

fn scrub_is_check(kind: &mut IsCheckKind) {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => scrub_value(value),
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::Normalized(_) => {}
    }
}

fn scrub_mutation_pipeline(pipeline: &mut MutationPipeline) {
    pipeline.span = SourceSpan::default();
    for statement in &mut pipeline.statements {
        match statement {
            MutationStatement::Match(value) => scrub_match(value),
            MutationStatement::Filter(value) => scrub_value(value),
            MutationStatement::Insert(value) => {
                value.span = SourceSpan::default();
                for pattern in &mut value.patterns {
                    scrub_graph_pattern(pattern);
                }
            }
            MutationStatement::Set(values) => {
                for value in values {
                    match value {
                        SetItem::Property { value, span, .. } => {
                            *span = SourceSpan::default();
                            scrub_value(value);
                        }
                        SetItem::PropertyMerge {
                            properties, span, ..
                        } => {
                            *span = SourceSpan::default();
                            for (_, value) in properties {
                                scrub_value(value);
                            }
                        }
                        SetItem::Label { span, .. } => *span = SourceSpan::default(),
                    }
                }
            }
            MutationStatement::Remove(values) => {
                for value in values {
                    match value {
                        crate::RemoveItem::Property { span, .. }
                        | crate::RemoveItem::Label { span, .. } => *span = SourceSpan::default(),
                    }
                }
            }
            MutationStatement::Delete(value) => value.span = SourceSpan::default(),
        }
    }
    if let Some(terminator) = &mut pipeline.terminator {
        match terminator {
            MutationTerminator::Return(value) => scrub_return(value),
            MutationTerminator::Finish(span) => *span = SourceSpan::default(),
        }
    }
}

fn scrub_ddl(statement: &mut DdlStatement) {
    match statement {
        DdlStatement::CreateGraph { span, .. }
        | DdlStatement::DropGraph { span, .. }
        | DdlStatement::DropNodeType { span, .. }
        | DdlStatement::DropEdgeType { span, .. }
        | DdlStatement::CreateIndex { span, .. }
        | DdlStatement::DropIndex { span, .. } => *span = SourceSpan::default(),
        DdlStatement::CreateNodeType {
            properties, span, ..
        }
        | DdlStatement::CreateEdgeType {
            properties, span, ..
        } => {
            *span = SourceSpan::default();
            for property in properties {
                scrub_property_def(property);
            }
        }
        DdlStatement::ShowNodeTypes(span)
        | DdlStatement::ShowEdgeTypes(span)
        | DdlStatement::ShowIndexes(span)
        | DdlStatement::ShowProcedures(span) => {
            *span = SourceSpan::default();
        }
    }
}

fn scrub_property_def(property: &mut TypePropertyDef) {
    property.span = SourceSpan::default();
    for constraint in &mut property.constraints {
        match constraint {
            TypePropertyConstraint::NotNull(span)
            | TypePropertyConstraint::Immutable(span)
            | TypePropertyConstraint::Unique(span)
            | TypePropertyConstraint::Searchable(span)
            | TypePropertyConstraint::Dictionary(span)
            | TypePropertyConstraint::Fill(_, span)
            | TypePropertyConstraint::Interval(_, span)
            | TypePropertyConstraint::Encoding(_, span) => *span = SourceSpan::default(),
            TypePropertyConstraint::Indexed { span, .. } => *span = SourceSpan::default(),
            TypePropertyConstraint::Default(value, span) => {
                *span = SourceSpan::default();
                scrub_value(value);
            }
        }
    }
}

fn scrub_call(call: &mut ProcedureCall) {
    call.span = SourceSpan::default();
    for arg in &mut call.args {
        scrub_value(arg);
    }
    for item in &mut call.yield_items {
        item.span = SourceSpan::default();
    }
}

fn scrub_inline_call(call: &mut InlineProcedureCall) {
    call.span = SourceSpan::default();
    scrub_query_pipeline(&mut call.body);
    for item in &mut call.yield_items {
        item.span = SourceSpan::default();
    }
}

#[cfg(test)]
mod tests {
    use crate::{Statement, parse};

    use super::structurally_eq;

    #[test]
    fn ignores_span_differences() {
        let left = parse("RETURN 1").expect("parse");
        let right = parse("\nRETURN 1").expect("parse");
        assert!(structurally_eq(&left, &right));
    }

    #[test]
    fn preserves_structural_differences() {
        let left = parse("RETURN 1").expect("parse");
        let right = parse("RETURN 2").expect("parse");
        assert!(!structurally_eq(&left, &right));
        assert!(matches!(left, Statement::Query(_)));
    }
}
