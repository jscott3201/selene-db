//! Propagate inline parameter declarations to bare references in the same statement.

use crate::{
    DdlStatement, IsCheckKind, LimitValue, MatchClause, MutationPipeline, MutationStatement,
    MutationTerminator, PatternElement, PipelineStatement, ProcedureCall, QueryPipeline,
    ReturnClause, ReturnItem, SetItem, Statement, TypePropertyConstraint, UnwindStatement,
    ValueExpr,
};

use super::parameters::DeclarationMap;

pub(super) fn inherit_statement_parameter_declarations(
    statement: &mut Statement,
    declarations: &DeclarationMap,
) {
    match statement {
        Statement::Query(pipeline) => {
            inherit_pipeline_parameter_declarations(pipeline, declarations);
        }
        Statement::Composite { first, rest, .. } => {
            inherit_pipeline_parameter_declarations(first, declarations);
            for (_, pipeline) in rest {
                inherit_pipeline_parameter_declarations(pipeline, declarations);
            }
        }
        Statement::Chained { blocks, .. } => {
            for pipeline in blocks {
                inherit_pipeline_parameter_declarations(pipeline, declarations);
            }
        }
        Statement::Mutate(pipeline) => {
            inherit_mutation_parameter_declarations(pipeline, declarations);
        }
        Statement::Ddl(statement) => {
            inherit_ddl_parameter_declarations(statement, declarations);
        }
        Statement::Call(call) => {
            inherit_call_parameter_declarations(call, declarations);
        }
        Statement::Explain { inner, .. } => {
            inherit_statement_parameter_declarations(inner, declarations);
        }
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::SessionSetValue { .. }
        | Statement::SessionSetTimeZone { .. }
        | Statement::SessionReset { .. }
        | Statement::SessionClose { .. } => {}
    }
}

fn inherit_pipeline_parameter_declarations(
    pipeline: &mut QueryPipeline,
    declarations: &DeclarationMap,
) {
    for statement in &mut pipeline.statements {
        match statement {
            PipelineStatement::Match(clause) => {
                inherit_match_clause_parameter_declarations(clause, declarations);
            }
            PipelineStatement::Filter(value)
            | PipelineStatement::Unwind(UnwindStatement { source: value, .. }) => {
                inherit_value_parameter_declarations(value, declarations);
            }
            PipelineStatement::Let(bindings) => {
                for binding in bindings {
                    inherit_value_parameter_declarations(&mut binding.value, declarations);
                }
            }
            PipelineStatement::Sorting(terms) => {
                for term in terms {
                    inherit_value_parameter_declarations(&mut term.expr, declarations);
                }
            }
            PipelineStatement::Limit(value) | PipelineStatement::Offset(value) => {
                inherit_limit_parameter_declarations(value, declarations);
            }
            PipelineStatement::Return(clause) => {
                inherit_return_parameter_declarations(clause, declarations);
            }
            PipelineStatement::With(clause) => {
                inherit_projection_parameter_declarations(
                    &mut clause.items,
                    clause.group_by.as_deref_mut(),
                    clause.having.as_mut(),
                    declarations,
                );
                if let Some(where_clause) = &mut clause.where_clause {
                    inherit_value_parameter_declarations(where_clause, declarations);
                }
            }
            PipelineStatement::Call(call) => {
                inherit_call_parameter_declarations(call, declarations);
            }
            PipelineStatement::CallSubquery(call) => {
                inherit_pipeline_parameter_declarations(&mut call.body, declarations);
            }
        }
    }
}

fn inherit_mutation_parameter_declarations(
    pipeline: &mut MutationPipeline,
    declarations: &DeclarationMap,
) {
    for statement in &mut pipeline.statements {
        match statement {
            MutationStatement::Match(clause) => {
                inherit_match_clause_parameter_declarations(clause, declarations);
            }
            MutationStatement::Filter(value) => {
                inherit_value_parameter_declarations(value, declarations);
            }
            MutationStatement::Insert(insert) => {
                for pattern in &mut insert.patterns {
                    inherit_graph_pattern_parameter_declarations(pattern, declarations);
                }
            }
            MutationStatement::Set(items) => {
                for item in items {
                    match item {
                        SetItem::Property { value, .. } => {
                            inherit_value_parameter_declarations(value, declarations);
                        }
                        SetItem::PropertyMerge { properties, .. } => {
                            for (_, value) in properties {
                                inherit_value_parameter_declarations(value, declarations);
                            }
                        }
                        SetItem::Label { .. } => {}
                    }
                }
            }
            MutationStatement::Remove(_) | MutationStatement::Delete(_) => {}
        }
    }
    if let Some(MutationTerminator::Return(clause)) = &mut pipeline.terminator {
        inherit_return_parameter_declarations(clause, declarations);
    }
}

fn inherit_ddl_parameter_declarations(statement: &mut DdlStatement, declarations: &DeclarationMap) {
    match statement {
        DdlStatement::CreateNodeType { properties, .. }
        | DdlStatement::CreateEdgeType { properties, .. } => {
            for property in properties {
                for constraint in &mut property.constraints {
                    if let TypePropertyConstraint::Default(value, _) = constraint {
                        inherit_value_parameter_declarations(value, declarations);
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
}

fn inherit_return_parameter_declarations(clause: &mut ReturnClause, declarations: &DeclarationMap) {
    inherit_projection_parameter_declarations(
        &mut clause.items,
        clause.group_by.as_deref_mut(),
        clause.having.as_mut(),
        declarations,
    );
}

fn inherit_projection_parameter_declarations(
    items: &mut [ReturnItem],
    group_by: Option<&mut [ValueExpr]>,
    having: Option<&mut ValueExpr>,
    declarations: &DeclarationMap,
) {
    for item in items {
        inherit_value_parameter_declarations(&mut item.expr, declarations);
    }
    if let Some(values) = group_by {
        for value in values {
            inherit_value_parameter_declarations(value, declarations);
        }
    }
    if let Some(value) = having {
        inherit_value_parameter_declarations(value, declarations);
    }
}

fn inherit_call_parameter_declarations(call: &mut ProcedureCall, declarations: &DeclarationMap) {
    for arg in &mut call.args {
        inherit_value_parameter_declarations(arg, declarations);
    }
    if let Some(filter) = &mut call.yield_filter {
        inherit_value_parameter_declarations(filter, declarations);
    }
}

fn inherit_match_clause_parameter_declarations(
    clause: &mut MatchClause,
    declarations: &DeclarationMap,
) {
    for pattern in &mut clause.patterns {
        inherit_graph_pattern_parameter_declarations(pattern, declarations);
    }
    if let Some(value) = &mut clause.where_clause {
        inherit_value_parameter_declarations(value, declarations);
    }
}

fn inherit_graph_pattern_parameter_declarations(
    pattern: &mut crate::GraphPattern,
    declarations: &DeclarationMap,
) {
    for element in &mut pattern.elements {
        match element {
            PatternElement::Node(node) => {
                for (_, value) in &mut node.properties {
                    inherit_value_parameter_declarations(value, declarations);
                }
                if let Some(value) = &mut node.inline_where {
                    inherit_value_parameter_declarations(value, declarations);
                }
            }
            PatternElement::Edge(edge) => {
                for (_, value) in &mut edge.properties {
                    inherit_value_parameter_declarations(value, declarations);
                }
                if let Some(value) = &mut edge.inline_where {
                    inherit_value_parameter_declarations(value, declarations);
                }
            }
        }
    }
}

fn inherit_limit_parameter_declarations(value: &mut LimitValue, declarations: &DeclarationMap) {
    if let LimitValue::Parameter {
        name,
        declared_type,
        ..
    } = value
        && declared_type.is_none()
        && let Some((ty, _)) = declarations.get(&*name)
    {
        *declared_type = Some(ty.clone());
    }
}

fn inherit_value_parameter_declarations(value: &mut ValueExpr, declarations: &DeclarationMap) {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            ValueExpr::Parameter {
                name,
                declared_type,
                ..
            } => {
                if declared_type.is_none()
                    && let Some((ty, _)) = declarations.get(&*name)
                {
                    *declared_type = Some(ty.clone());
                }
            }
            ValueExpr::PropertyAccess { target, .. }
            | ValueExpr::UnaryOp {
                operand: target, ..
            }
            | ValueExpr::PropertyExists { target, .. }
            | ValueExpr::Normalize { source: target, .. }
            | ValueExpr::Cast { value: target, .. } => stack.push(target.as_mut()),
            ValueExpr::ListAccess { target, index, .. } => {
                stack.push(index.as_mut());
                stack.push(target.as_mut());
            }
            ValueExpr::ListLiteral { items, .. }
            | ValueExpr::AllDifferent { items, .. }
            | ValueExpr::Same { items, .. }
            | ValueExpr::FunctionCall { args: items, .. } => stack.extend(items.iter_mut()),
            ValueExpr::RecordLiteral { fields, .. } => {
                stack.extend(fields.iter_mut().map(|(_, value)| value));
            }
            ValueExpr::BinaryOp { lhs, rhs, .. } => {
                stack.push(rhs.as_mut());
                stack.push(lhs.as_mut());
            }
            ValueExpr::IsCheck { operand, kind, .. } => {
                stack.push(operand.as_mut());
                match kind {
                    IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
                        stack.push(value.as_mut());
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
                stack.extend(list.iter_mut());
                stack.push(operand.as_mut());
            }
            ValueExpr::Case {
                branches,
                else_branch,
                ..
            } => {
                if let Some(value) = else_branch {
                    stack.push(value.as_mut());
                }
                for (condition, result) in branches {
                    stack.push(result);
                    stack.push(condition);
                }
            }
            ValueExpr::Trim {
                character, source, ..
            } => {
                stack.push(source.as_mut());
                if let Some(character) = character {
                    stack.push(character.as_mut());
                }
            }
            ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
                inherit_match_clause_parameter_declarations(pattern, declarations);
            }
            ValueExpr::ValueSubquery { body, .. } => {
                inherit_pipeline_parameter_declarations(body, declarations);
            }
            ValueExpr::Literal(_) | ValueExpr::Variable { .. } => {}
        }
    }
}
