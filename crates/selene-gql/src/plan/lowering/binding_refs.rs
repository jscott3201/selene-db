//! Binding-reference collection for planned expressions.

use crate::{
    SourceSpan, ValueExpr,
    analyze::{AnalyzedStatement, BindingId},
    plan::PlannerError,
};

use super::expr::{outer_binding_uses_in_match, outer_binding_uses_in_span};

/// Bindings referenced by `expr`, sorted and deduplicated.
pub(crate) fn binding_refs_in(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<BindingId>, PlannerError> {
    let mut refs = Vec::new();
    collect_binding_refs_in_expr(expr, analyzed, &mut refs)?;
    refs.sort_by_key(|(binding, _)| *binding);
    refs.dedup_by_key(|(binding, _)| *binding);
    refs.into_iter()
        .map(|(binding, span)| {
            ensure_binding_exists(binding, span, analyzed)?;
            Ok(binding)
        })
        .collect()
}

fn collect_binding_refs_in_expr(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
    refs: &mut Vec<(BindingId, SourceSpan)>,
) -> Result<(), PlannerError> {
    match expr {
        // A `Variable` resolves to the binding(s) recorded for its exact
        // name+span by the analyzer; this is leaf work, not child recursion.
        ValueExpr::Variable { name, span } => {
            refs.extend(
                analyzed
                    .references
                    .iter()
                    .filter(|reference| reference.name == *name && reference.span == *span)
                    .map(|reference| (reference.binding, *span)),
            );
        }
        // Subquery bodies are `MatchClause` / `QueryPipeline`, not `ValueExpr`
        // children: collect the outer-binding uses they reference rather than
        // recursing through `for_each_child`.
        ValueExpr::Exists { pattern, span, .. } => {
            refs.extend(
                outer_binding_uses_in_match(pattern, *span, analyzed)?
                    .into_iter()
                    .map(|(binding, _, span)| (binding, span)),
            );
        }
        ValueExpr::ValueSubquery { body, .. } => {
            refs.extend(
                outer_binding_uses_in_span(body.span, body.span, analyzed)?
                    .into_iter()
                    .map(|(binding, _, span)| (binding, span)),
            );
        }
        // Every other variant only recurses into its direct `ValueExpr` children
        // (including the `IS [SOURCE|DESTINATION] OF` operand, so collected refs
        // include the edge it binds).
        _ => {
            let mut result = Ok(());
            expr.for_each_child(&mut |child| {
                if result.is_ok() {
                    result = collect_binding_refs_in_expr(child, analyzed, refs);
                }
            });
            result?;
        }
    }
    Ok(())
}

fn ensure_binding_exists(
    binding: BindingId,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<(), PlannerError> {
    analyzed
        .scopes
        .declaration(binding)
        .map(|_| ())
        .ok_or(PlannerError::BindingResolutionLost { binding, span })
}
