//! Typed parameter declaration checks.

use std::collections::BTreeMap;

use selene_core::DbString;

use crate::{
    DdlStatement, ForStatement, GqlType, IsCheckKind, LimitValue, MatchClause, MutationPipeline,
    MutationStatement, MutationTerminator, PatternElement, PipelineStatement, ProcedureCall,
    QueryPipeline, ReturnClause, ReturnItem, SetItem, SourceSpan, Statement,
    TypePropertyConstraint, ValueExpr, analyze::error::AnalysisError,
};

pub(super) type DeclarationMap = BTreeMap<DbString, (GqlType, SourceSpan)>;

pub(crate) fn apply_statement_parameter_declarations(
    statement: &mut Statement,
) -> Result<(), AnalysisError> {
    let mut declarations = BTreeMap::new();
    collect_statement_parameter_declarations(statement, &mut declarations)?;
    if !declarations.is_empty() {
        super::parameter_inheritance::inherit_statement_parameter_declarations(
            statement,
            &declarations,
        );
    }
    Ok(())
}

pub(crate) fn validate_parameter_declarations(
    pipeline: &QueryPipeline,
) -> Result<(), AnalysisError> {
    let mut declarations = BTreeMap::new();
    collect_pipeline_parameter_declarations(pipeline, &mut declarations)
}

fn collect_statement_parameter_declarations(
    statement: &Statement,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    match statement {
        Statement::Query(pipeline) => {
            collect_pipeline_parameter_declarations(pipeline, declarations)
        }
        Statement::Composite { first, rest, .. } => {
            collect_pipeline_parameter_declarations(first, declarations)?;
            for (_, pipeline) in rest {
                collect_pipeline_parameter_declarations(pipeline, declarations)?;
            }
            Ok(())
        }
        Statement::Chained { blocks, .. } => {
            for pipeline in blocks {
                collect_pipeline_parameter_declarations(pipeline, declarations)?;
            }
            Ok(())
        }
        Statement::Mutate(pipeline) => {
            collect_mutation_parameter_declarations(pipeline, declarations)
        }
        Statement::Ddl(statement) => collect_ddl_parameter_declarations(statement, declarations),
        Statement::Call(call) => collect_call_parameter_declarations(call, declarations),
        Statement::Explain { inner, .. } => {
            collect_statement_parameter_declarations(inner, declarations)
        }
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::SessionSetValue { .. }
        | Statement::SessionSetTimeZone { .. }
        | Statement::SessionReset { .. }
        | Statement::SessionClose { .. } => Ok(()),
    }
}

fn collect_pipeline_parameter_declarations(
    pipeline: &QueryPipeline,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    for statement in &pipeline.statements {
        match statement {
            PipelineStatement::Match(clause) => {
                collect_match_clause_parameter_declarations(clause, declarations)?;
            }
            PipelineStatement::Filter(value)
            | PipelineStatement::For(ForStatement { source: value, .. }) => {
                collect_value_parameter_declarations(value, declarations)?;
            }
            PipelineStatement::Let(bindings) => {
                for binding in bindings {
                    collect_value_parameter_declarations(&binding.value, declarations)?;
                }
            }
            PipelineStatement::Sorting(terms) => {
                for term in terms {
                    collect_value_parameter_declarations(&term.expr, declarations)?;
                }
            }
            PipelineStatement::Limit(value) | PipelineStatement::Offset(value) => {
                collect_limit_parameter_declarations(value, declarations)?;
            }
            PipelineStatement::Return(clause) => {
                collect_return_parameter_declarations(clause, declarations)?;
            }
            PipelineStatement::With(clause) => {
                collect_projection_parameter_declarations(
                    &clause.items,
                    clause.group_by.as_deref(),
                    clause.having.as_ref(),
                    declarations,
                )?;
                if let Some(where_clause) = &clause.where_clause {
                    collect_value_parameter_declarations(where_clause, declarations)?;
                }
            }
            PipelineStatement::Call(call) => {
                collect_call_parameter_declarations(call, declarations)?;
            }
            PipelineStatement::CallSubquery(call) => {
                collect_pipeline_parameter_declarations(&call.body, declarations)?;
            }
        }
    }
    Ok(())
}

fn collect_mutation_parameter_declarations(
    pipeline: &MutationPipeline,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    for statement in &pipeline.statements {
        match statement {
            MutationStatement::Match(clause) => {
                collect_match_clause_parameter_declarations(clause, declarations)?;
            }
            MutationStatement::Filter(value) => {
                collect_value_parameter_declarations(value, declarations)?;
            }
            MutationStatement::Insert(insert) => {
                for pattern in &insert.patterns {
                    collect_graph_pattern_parameter_declarations(pattern, declarations)?;
                }
            }
            MutationStatement::Set(items) => {
                for item in items {
                    match item {
                        SetItem::Property { value, .. } => {
                            collect_value_parameter_declarations(value, declarations)?;
                        }
                        SetItem::PropertyMerge { properties, .. } => {
                            for (_, value) in properties {
                                collect_value_parameter_declarations(value, declarations)?;
                            }
                        }
                        SetItem::Label { .. } => {}
                    }
                }
            }
            MutationStatement::Remove(_) | MutationStatement::Delete(_) => {}
        }
    }
    if let Some(MutationTerminator::Return(clause)) = &pipeline.terminator {
        collect_return_parameter_declarations(clause, declarations)?;
    }
    Ok(())
}

fn collect_ddl_parameter_declarations(
    statement: &DdlStatement,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    match statement {
        DdlStatement::CreateNodeType { properties, .. }
        | DdlStatement::CreateEdgeType { properties, .. } => {
            for property in properties {
                for constraint in &property.constraints {
                    if let TypePropertyConstraint::Default(value, _) = constraint {
                        collect_value_parameter_declarations(value, declarations)?;
                    }
                }
            }
        }
        DdlStatement::CreateGraph { .. }
        | DdlStatement::DropGraph { .. }
        | DdlStatement::DropNodeType { .. }
        | DdlStatement::DropEdgeType { .. }
        | DdlStatement::TruncateNodeType { .. }
        | DdlStatement::TruncateEdgeType { .. }
        | DdlStatement::CreateIndex { .. }
        | DdlStatement::DropIndex { .. }
        | DdlStatement::ShowNodeTypes(_)
        | DdlStatement::ShowEdgeTypes(_)
        | DdlStatement::ShowIndexes(_)
        | DdlStatement::ShowProcedures(_) => {}
    }
    Ok(())
}

fn collect_return_parameter_declarations(
    clause: &ReturnClause,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    collect_projection_parameter_declarations(
        &clause.items,
        clause.group_by.as_deref(),
        clause.having.as_ref(),
        declarations,
    )
}

fn collect_projection_parameter_declarations(
    items: &[ReturnItem],
    group_by: Option<&[ValueExpr]>,
    having: Option<&ValueExpr>,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    for item in items {
        collect_value_parameter_declarations(&item.expr, declarations)?;
    }
    if let Some(values) = group_by {
        for value in values {
            collect_value_parameter_declarations(value, declarations)?;
        }
    }
    if let Some(value) = having {
        collect_value_parameter_declarations(value, declarations)?;
    }
    Ok(())
}

fn collect_call_parameter_declarations(
    call: &ProcedureCall,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    for arg in &call.args {
        collect_value_parameter_declarations(arg, declarations)?;
    }
    if let Some(filter) = &call.yield_filter {
        collect_value_parameter_declarations(filter, declarations)?;
    }
    Ok(())
}

fn collect_match_clause_parameter_declarations(
    clause: &MatchClause,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    for pattern in &clause.patterns {
        collect_graph_pattern_parameter_declarations(pattern, declarations)?;
    }
    if let Some(value) = &clause.where_clause {
        collect_value_parameter_declarations(value, declarations)?;
    }
    Ok(())
}

fn collect_graph_pattern_parameter_declarations(
    pattern: &crate::GraphPattern,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => {
                for (_, value) in &node.properties {
                    collect_value_parameter_declarations(value, declarations)?;
                }
                if let Some(value) = &node.inline_where {
                    collect_value_parameter_declarations(value, declarations)?;
                }
            }
            PatternElement::Edge(edge) => {
                for (_, value) in &edge.properties {
                    collect_value_parameter_declarations(value, declarations)?;
                }
                if let Some(value) = &edge.inline_where {
                    collect_value_parameter_declarations(value, declarations)?;
                }
            }
        }
    }
    Ok(())
}

fn collect_limit_parameter_declarations(
    value: &LimitValue,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    if let LimitValue::Parameter {
        name,
        declared_type: Some(declared_type),
        span,
    } = value
    {
        record_parameter_declaration(declarations, name.clone(), declared_type, *span)?;
    }
    Ok(())
}

fn collect_value_parameter_declarations(
    value: &ValueExpr,
    declarations: &mut DeclarationMap,
) -> Result<(), AnalysisError> {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            ValueExpr::Parameter {
                name,
                declared_type: Some(declared_type),
                span,
            } => {
                record_parameter_declaration(declarations, name.clone(), declared_type, *span)?;
            }
            ValueExpr::PropertyAccess { target, .. }
            | ValueExpr::UnaryOp {
                operand: target, ..
            }
            | ValueExpr::PropertyExists { target, .. }
            | ValueExpr::Normalize { source: target, .. }
            | ValueExpr::Cast { value: target, .. } => stack.push(target),
            ValueExpr::ListLiteral { items, .. }
            | ValueExpr::PathConstructor {
                elements: items, ..
            }
            | ValueExpr::AllDifferent { items, .. }
            | ValueExpr::Same { items, .. }
            | ValueExpr::FunctionCall { args: items, .. } => stack.extend(items.iter()),
            ValueExpr::DurationBetween { start, end, .. } => {
                stack.push(end);
                stack.push(start);
            }
            ValueExpr::RecordLiteral { fields, .. } => {
                stack.extend(fields.iter().map(|(_, value)| value));
            }
            ValueExpr::BinaryOp { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            ValueExpr::IsCheck { operand, kind, .. } => {
                stack.push(operand);
                match kind {
                    IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
                        stack.push(value);
                    }
                    IsCheckKind::Null
                    | IsCheckKind::Directed
                    | IsCheckKind::Labeled(_)
                    | IsCheckKind::TruthValue(_)
                    | IsCheckKind::Typed(_)
                    | IsCheckKind::Normalized(_) => {}
                }
            }
            ValueExpr::InList { operand, list, .. } => {
                stack.extend(list.iter());
                stack.push(operand);
            }
            ValueExpr::InListExpression { operand, list, .. } => {
                stack.push(list);
                stack.push(operand);
            }
            ValueExpr::Case {
                branches,
                else_branch,
                ..
            } => {
                if let Some(value) = else_branch {
                    stack.push(value);
                }
                for (condition, result) in branches {
                    stack.push(result);
                    stack.push(condition);
                }
            }
            ValueExpr::Trim {
                character, source, ..
            } => {
                stack.push(source);
                if let Some(character) = character {
                    stack.push(character);
                }
            }
            ValueExpr::Exists { pattern, .. } => {
                collect_match_clause_parameter_declarations(pattern, declarations)?;
            }
            ValueExpr::ValueSubquery { body, .. } => {
                collect_pipeline_parameter_declarations(body, declarations)?;
            }
            ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        }
    }
    Ok(())
}

fn record_parameter_declaration(
    declarations: &mut DeclarationMap,
    name: DbString,
    declared_type: &GqlType,
    span: SourceSpan,
) -> Result<(), AnalysisError> {
    if let Some((prior_type, prior_span)) = declarations.get(&name) {
        if prior_type != declared_type {
            return Err(AnalysisError::ConflictingParameterTypes {
                name,
                declarations: vec![
                    (prior_type.clone(), *prior_span),
                    (declared_type.clone(), span),
                ],
            });
        }
    } else {
        declarations.insert(name, (declared_type.clone(), span));
    }
    Ok(())
}
