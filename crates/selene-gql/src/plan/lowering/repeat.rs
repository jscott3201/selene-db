//! Variable-length edge pattern lowering helpers.

use crate::{
    EdgePattern,
    plan::{PlannerError, RepeatEdgeMatch, ScanAccess, TailBinding},
};

use super::{
    expr,
    match_clause::{EdgeLoweringContext, RightNode, edge_binding},
};

pub(super) fn ensure_within_max_quantifier(
    max: u32,
    limit: u32,
    span: crate::SourceSpan,
) -> Result<(), PlannerError> {
    if max > limit {
        return Err(PlannerError::ProgramLimitExceeded {
            limit_name: "max_quantifier",
            limit,
            actual: max,
            span,
        });
    }
    Ok(())
}

pub(super) fn edge_match(
    edge: &EdgePattern,
    left_binding: Option<TailBinding>,
    final_node: RightNode,
    ctx: &mut EdgeLoweringContext<'_, '_>,
) -> Result<RepeatEdgeMatch, PlannerError> {
    let group_binding = edge_binding(edge, ctx.analyzed, ctx.names, ctx.binding_ids)?;
    let property_predicates = edge
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(None, key.clone(), value, ctx.analyzed))
        .collect::<Result<Vec<_>, _>>()?;
    let inline_predicates = edge
        .inline_where
        .as_ref()
        .map(|where_clause| expr::filter_predicate(where_clause, ctx.analyzed))
        .transpose()?
        .into_iter()
        .collect();
    Ok(RepeatEdgeMatch {
        group_binding,
        group_hidden_binding: None,
        label_predicate: edge.label_expr.clone(),
        property_predicates,
        inline_predicates,
        left_binding: left_binding.and_then(TailBinding::named),
        left_hidden_binding: left_binding.and_then(TailBinding::hidden),
        final_binding: final_node.binding,
        final_hidden_binding: final_node.hidden_binding,
        final_label_predicate: final_node.label_predicate,
        final_property_predicates: final_node.property_predicates,
        access: ScanAccess::Linear,
        span: edge.span,
    })
}
