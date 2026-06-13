//! Read-query bind handling.

use std::collections::BTreeSet;

use crate::{
    GqlType, InlineProcedureCall, LetBinding, LimitValue, OrderTerm, PipelineStatement,
    ProcedureMutability, QueryPipeline, ReturnClause, ReturnItem, SourceSpan, UnwindStatement,
    ValueExpr, WithClause, YieldColumn,
    analyze::{
        BindingDeclKind, BindingId, ScopeId,
        error::{AnalysisError, ConditionClause, ExpectedType, TypeMismatchContext},
        types::AnalyzedType,
    },
};

use super::{BindContext, call, expr, expr_depth, pattern};
use crate::analyze::bind::aggregate_rules;
use crate::analyze::scope::ScopeKind;

pub(crate) fn bind_query_pipeline(
    ctx: &mut BindContext,
    pipeline: &mut QueryPipeline,
) -> Result<(), AnalysisError> {
    super::parameters::validate_parameter_declarations(pipeline)?;
    for statement in &mut pipeline.statements {
        bind_pipeline_statement(ctx, statement)?;
    }
    Ok(())
}

pub(crate) fn bind_pipeline_statement(
    ctx: &mut BindContext,
    statement: &mut PipelineStatement,
) -> Result<(), AnalysisError> {
    match statement {
        PipelineStatement::Match(clause) => pattern::bind_match_clause(ctx, clause),
        PipelineStatement::Filter(value) => {
            expr::bind_condition(ctx, value, ConditionClause::Filter)?;
            Ok(())
        }
        PipelineStatement::Let(bindings) => bind_let(ctx, bindings),
        PipelineStatement::Unwind(unwind) => bind_unwind(ctx, unwind),
        PipelineStatement::Sorting(terms) => bind_sorting(ctx, terms),
        PipelineStatement::Limit(value) | PipelineStatement::Offset(value) => {
            bind_limit_value(value)
        }
        PipelineStatement::Return(clause) => bind_return_clause(ctx, clause),
        PipelineStatement::With(clause) => bind_with_clause(ctx, clause),
        PipelineStatement::Call(call) => {
            let metadata = call::lookup_metadata(ctx, call)?;
            if matches!(
                metadata.mutability,
                ProcedureMutability::SchemaWrite | ProcedureMutability::MaintenanceWrite
            ) {
                return Err(AnalysisError::MutatingProcedureInReadPipeline {
                    procedure: call.name.clone().into_vec().into_boxed_slice(),
                    mutability: metadata.mutability,
                    span: call.span,
                });
            }
            call::bind_procedure_call_with_metadata(ctx, call, metadata)?;
            Ok(())
        }
        PipelineStatement::CallSubquery(call) => bind_inline_call(ctx, call),
    }
}

fn bind_inline_call(
    ctx: &mut BindContext,
    call: &mut InlineProcedureCall,
) -> Result<(), AnalysisError> {
    if call.in_transactions {
        return Err(AnalysisError::NotImplemented {
            message: "CALL { ... } IN TRANSACTIONS is not yet supported".into(),
            span: call.span,
            hint: None,
        });
    }
    expr_depth::check_query_subquery_depth(&call.body, 1)?;
    let bind_result = match &call.variable_scope {
        // GP03 (ISO §15.2): explicit variable scope — the body sees ONLY the
        // named imports. An empty list (`CALL () { ... }`) is fully isolated.
        // Imports are cloned so the body's `&mut call.body` borrow stays disjoint
        // from the `call.variable_scope` read.
        Some(imports) => {
            let imports = imports.clone();
            ctx.with_imported_scope(&imports, call.span, |ctx| {
                bind_query_pipeline(ctx, &mut call.body)
            })
        }
        // GP02: implicit scope — the body inherits all outer bindings.
        None => ctx.with_child_scope(ScopeKind::Subquery, call.span, false, |ctx| {
            bind_query_pipeline(ctx, &mut call.body)
        }),
    };
    if let Err(AnalysisError::MutatingProcedureInReadPipeline { span, .. }) = bind_result {
        return Err(AnalysisError::NotImplemented {
            message: "write operations inside CALL { ... } are not yet supported".into(),
            span,
            hint: None,
        });
    }
    bind_result?;
    let outputs = inline_call_outputs(ctx, &call.body)?;
    declare_inline_call_yields(ctx, call, &outputs)
}

#[derive(Clone)]
struct InlineCallOutput {
    name: selene_core::DbString,
    ty: AnalyzedType,
    span: crate::SourceSpan,
}

fn inline_call_outputs(
    ctx: &BindContext,
    body: &QueryPipeline,
) -> Result<Vec<InlineCallOutput>, AnalysisError> {
    let Some(return_clause) = body.statements.iter().rev().find_map(|statement| {
        if let PipelineStatement::Return(clause) = statement {
            Some(clause)
        } else {
            None
        }
    }) else {
        return Ok(Vec::new());
    };
    if return_clause.star {
        return Ok(Vec::new());
    }
    Ok(return_clause
        .items
        .iter()
        .filter_map(|item| {
            projection_name(item).map(|name| {
                let ty = ctx
                    .expr_id(&item.expr)
                    .map(|id| ctx.expr_type(id).clone())
                    .unwrap_or(AnalyzedType::Dynamic);
                InlineCallOutput {
                    name,
                    ty,
                    span: item.span,
                }
            })
        })
        .collect::<Vec<_>>())
}

fn declare_inline_call_yields(
    ctx: &mut BindContext,
    call: &InlineProcedureCall,
    outputs: &[InlineCallOutput],
) -> Result<(), AnalysisError> {
    if call.yield_items.is_empty() {
        return Ok(());
    }
    for item in &call.yield_items {
        match &item.column {
            YieldColumn::Star => {
                for output in outputs {
                    ctx.declare_strict_typed(
                        BindingDeclKind::YieldColumn,
                        output.name.clone(),
                        item.span,
                        output.ty.clone(),
                    )?;
                }
            }
            YieldColumn::Named(column) => {
                let Some(output) = outputs.iter().find(|output| output.name == *column) else {
                    return Err(AnalysisError::UnknownYieldColumn {
                        procedure: Box::new([]),
                        column: column.clone(),
                        span: item.span,
                    });
                };
                ctx.declare_strict_typed(
                    BindingDeclKind::YieldColumn,
                    item.alias.clone().unwrap_or_else(|| column.clone()),
                    output.span,
                    output.ty.clone(),
                )?;
            }
        }
    }
    Ok(())
}

fn projection_name(item: &ReturnItem) -> Option<selene_core::DbString> {
    item.alias.clone().or({
        if let ValueExpr::Variable { name, .. } = &item.expr {
            Some(name.clone())
        } else {
            None
        }
    })
}

pub(crate) fn bind_return_clause(
    ctx: &mut BindContext,
    clause: &ReturnClause,
) -> Result<(), AnalysisError> {
    bind_return_inputs(
        ctx,
        &clause.items,
        clause.group_by.as_deref(),
        clause.having.as_ref(),
    )?;
    // Non-boundary projection scope: ISO GA07 lets ORDER BY / OFFSET /
    // LIMIT reach the pre-RETURN bindings, and `RETURN *` keeps the whole
    // input row visible by virtue of the parent walk.
    ctx.enter_projection_scope(clause.span, false);
    declare_projection_items(ctx, &clause.items)
}

fn bind_with_clause(ctx: &mut BindContext, clause: &WithClause) -> Result<(), AnalysisError> {
    bind_return_inputs(
        ctx,
        &clause.items,
        clause.group_by.as_deref(),
        clause.having.as_ref(),
    )?;
    // Boundary projection scope: pre-WITH bindings end here. Post-WITH
    // clauses see only the projection aliases declared below.
    ctx.enter_projection_scope(clause.span, true);
    declare_projection_items(ctx, &clause.items)?;
    if let Some(where_clause) = &clause.where_clause {
        expr::bind_condition(ctx, where_clause, ConditionClause::WithWhere)?;
    }
    Ok(())
}

fn bind_return_inputs(
    ctx: &mut BindContext,
    items: &[ReturnItem],
    group_by: Option<&[ValueExpr]>,
    having: Option<&ValueExpr>,
) -> Result<(), AnalysisError> {
    let row_scope = ctx.current_scope();
    for item in items {
        expr::bind_value_expr(ctx, &item.expr)?;
    }
    if let Some(values) = group_by {
        for value in values {
            expr::bind_value_expr(ctx, value)?;
        }
    }
    if let Some(value) = having {
        expr::bind_condition(ctx, value, ConditionClause::Having)?;
    }
    aggregate_rules::validate_aggregate_nesting(items, having)?;
    validate_percentile_independent_refs(ctx, row_scope, items, group_by, having)?;
    Ok(())
}

fn validate_percentile_independent_refs(
    ctx: &BindContext<'_>,
    row_scope: ScopeId,
    items: &[ReturnItem],
    group_by: Option<&[ValueExpr]>,
    having: Option<&ValueExpr>,
) -> Result<(), AnalysisError> {
    let group_bindings = group_binding_refs(ctx, group_by);
    for item in items {
        validate_percentile_independent_refs_in_expr(ctx, row_scope, &group_bindings, &item.expr)?;
    }
    if let Some(having) = having {
        validate_percentile_independent_refs_in_expr(ctx, row_scope, &group_bindings, having)?;
    }
    Ok(())
}

fn group_binding_refs(
    ctx: &BindContext<'_>,
    group_by: Option<&[ValueExpr]>,
) -> BTreeSet<BindingId> {
    let mut bindings = BTreeSet::new();
    if let Some(values) = group_by {
        for value in values {
            if let ValueExpr::Variable { span, .. } = value {
                bindings.extend(
                    binding_refs_in_span(ctx, *span)
                        .filter(|use_| use_.span == *span)
                        .map(|use_| use_.binding),
                );
            }
        }
    }
    bindings
}

fn validate_percentile_independent_refs_in_expr(
    ctx: &BindContext<'_>,
    row_scope: ScopeId,
    group_bindings: &BTreeSet<BindingId>,
    value: &ValueExpr,
) -> Result<(), AnalysisError> {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            ValueExpr::FunctionCall { name, args, .. }
                if name.len() == 1
                    && matches!(name.first().as_str(), "percentile_cont" | "percentile_disc") =>
            {
                if let Some(independent) = args.get(1) {
                    validate_percentile_independent_arg(
                        ctx,
                        row_scope,
                        group_bindings,
                        independent,
                    )?;
                }
                if let Some(dependent) = args.first() {
                    stack.push(dependent);
                }
            }
            ValueExpr::PropertyAccess { target, .. }
            | ValueExpr::UnaryOp {
                operand: target, ..
            }
            | ValueExpr::PropertyExists { target, .. }
            | ValueExpr::Cast { value: target, .. }
            | ValueExpr::Normalize { source: target, .. } => stack.push(target),
            ValueExpr::ListAccess { target, index, .. } => {
                stack.push(index);
                stack.push(target);
            }
            ValueExpr::ListLiteral { items, .. }
            | ValueExpr::PathConstructor {
                elements: items, ..
            }
            | ValueExpr::AllDifferent { items, .. }
            | ValueExpr::Same { items, .. }
            | ValueExpr::FunctionCall { args: items, .. } => {
                stack.extend(items.iter());
            }
            ValueExpr::DurationBetween { start, end, .. } => {
                stack.push(end);
                stack.push(start);
            }
            ValueExpr::RecordLiteral { fields, .. } => {
                stack.extend(fields.iter().map(|(_, field)| field));
            }
            ValueExpr::BinaryOp { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            ValueExpr::IsCheck { operand, kind, .. } => {
                stack.push(operand);
                match kind {
                    crate::IsCheckKind::SourceOf(value)
                    | crate::IsCheckKind::DestinationOf(value) => stack.push(value),
                    crate::IsCheckKind::Null
                    | crate::IsCheckKind::Directed
                    | crate::IsCheckKind::Labeled(_)
                    | crate::IsCheckKind::TruthValue(_)
                    | crate::IsCheckKind::Typed(_)
                    | crate::IsCheckKind::Normalized(_) => {}
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
            ValueExpr::Literal(_)
            | ValueExpr::Variable { .. }
            | ValueExpr::Parameter { .. }
            | ValueExpr::Exists { .. }
            | ValueExpr::CountSubquery { .. }
            | ValueExpr::ValueSubquery { .. } => {}
        }
    }
    Ok(())
}

fn validate_percentile_independent_arg(
    ctx: &BindContext<'_>,
    row_scope: ScopeId,
    group_bindings: &BTreeSet<BindingId>,
    independent: &ValueExpr,
) -> Result<(), AnalysisError> {
    for reference in binding_refs_in_span(ctx, independent.span()) {
        if group_bindings.contains(&reference.binding)
            || binding_declared_outside_current_subquery(ctx, row_scope, reference.binding)
        {
            continue;
        }
        return Err(AnalysisError::InvalidReference {
            message: "PERCENTILE independent expression cannot reference a per-row binding unless that binding is part of GROUP BY".to_owned(),
            span: reference.span,
        });
    }
    Ok(())
}

fn binding_refs_in_span<'ctx>(
    ctx: &'ctx BindContext<'_>,
    span: SourceSpan,
) -> impl Iterator<Item = &'ctx crate::analyze::BindingUse> {
    ctx.references
        .iter()
        .filter(move |reference| span_contains(span, reference.span))
}

fn binding_declared_outside_current_subquery(
    ctx: &BindContext<'_>,
    row_scope: ScopeId,
    binding: BindingId,
) -> bool {
    let Some(subquery_scope) = nearest_subquery_scope(ctx, row_scope) else {
        return false;
    };
    let Some(declaration_scope) = declaration_scope(ctx, binding) else {
        return false;
    };
    !scope_is_descendant_of(ctx, declaration_scope, subquery_scope)
}

fn nearest_subquery_scope(ctx: &BindContext<'_>, scope: ScopeId) -> Option<ScopeId> {
    let mut cursor = Some(scope);
    while let Some(scope_id) = cursor {
        let scope = ctx.scopes.scope(scope_id)?;
        if scope.kind == ScopeKind::Subquery {
            return Some(scope_id);
        }
        cursor = scope.parent;
    }
    None
}

fn declaration_scope(ctx: &BindContext<'_>, binding: BindingId) -> Option<ScopeId> {
    ctx.scopes
        .scopes()
        .iter()
        .enumerate()
        .find_map(|(index, scope)| {
            scope
                .locals
                .contains(&binding)
                .then(|| ScopeId::new(index as u32))
        })
}

fn scope_is_descendant_of(ctx: &BindContext<'_>, scope: ScopeId, ancestor: ScopeId) -> bool {
    let mut cursor = Some(scope);
    while let Some(scope_id) = cursor {
        if scope_id == ancestor {
            return true;
        }
        cursor = ctx.scopes.scope(scope_id).and_then(|scope| scope.parent);
    }
    false
}

fn span_contains(outer: SourceSpan, inner: SourceSpan) -> bool {
    outer.byte_offset <= inner.byte_offset && inner.end() <= outer.end()
}

fn declare_projection_items(
    ctx: &mut BindContext,
    items: &[ReturnItem],
) -> Result<(), AnalysisError> {
    for item in items {
        let ty = projection_item_type(ctx, item);
        if let Some(alias) = &item.alias {
            ctx.declare_strict_typed(
                BindingDeclKind::ProjectionAlias,
                alias.clone(),
                item.span,
                ty,
            )?;
        } else if let ValueExpr::Variable { name, span } = &item.expr {
            ctx.declare_strict_typed(BindingDeclKind::ProjectionAlias, name.clone(), *span, ty)?;
        }
    }
    Ok(())
}

fn bind_let(ctx: &mut BindContext, bindings: &[LetBinding]) -> Result<(), AnalysisError> {
    for binding in bindings {
        let id = expr::bind_value_expr(ctx, &binding.value)?;
        let ty = ctx.expr_type(id).clone();
        ctx.declare_strict_typed(
            BindingDeclKind::LetAlias,
            binding.alias.clone(),
            binding.span,
            ty,
        )?;
    }
    Ok(())
}

fn bind_unwind(ctx: &mut BindContext, unwind: &UnwindStatement) -> Result<(), AnalysisError> {
    let id = expr::bind_value_expr(ctx, &unwind.source)?;
    let ty = match ctx.expr_type(id) {
        AnalyzedType::Resolved(crate::GqlType::List(inner)) => {
            AnalyzedType::Resolved((**inner).clone())
        }
        _ => AnalyzedType::Dynamic,
    };
    ctx.declare_strict_typed(
        BindingDeclKind::UnwindAlias,
        unwind.alias.clone(),
        unwind.span,
        ty,
    )?;
    Ok(())
}

fn bind_sorting(ctx: &mut BindContext, terms: &[OrderTerm]) -> Result<(), AnalysisError> {
    for term in terms {
        expr::bind_value_expr(ctx, &term.expr)?;
    }
    Ok(())
}

fn bind_limit_value(value: &LimitValue) -> Result<(), AnalysisError> {
    match value {
        LimitValue::Count(..) => Ok(()),
        LimitValue::Parameter {
            declared_type: Some(declared_type),
            span,
            ..
        } if !is_limit_amount_type(declared_type) => Err(AnalysisError::TypeMismatch {
            context: TypeMismatchContext::LimitAmount,
            expected: ExpectedType::LimitAmount,
            found: declared_type.clone(),
            span: *span,
        }),
        LimitValue::Parameter { .. } => Ok(()),
    }
}

fn is_limit_amount_type(ty: &GqlType) -> bool {
    matches!(
        ty.strip_not_null(),
        GqlType::Integer
            | GqlType::Int8
            | GqlType::Int16
            | GqlType::Int32
            | GqlType::Int64
            | GqlType::SmallInt
            | GqlType::BigInt
            | GqlType::Int128
            | GqlType::Decimal
            | GqlType::DecimalExact(_)
            | GqlType::Uint8
            | GqlType::Uint16
            | GqlType::Uint32
            | GqlType::Uint64
            | GqlType::USmallInt
            | GqlType::Uint
            | GqlType::UBigInt
            | GqlType::Uint128
    )
}

fn projection_item_type(ctx: &BindContext, item: &ReturnItem) -> AnalyzedType {
    ctx.expr_id(&item.expr)
        .map(|id| ctx.expr_type(id).clone())
        .unwrap_or(AnalyzedType::Dynamic)
}
