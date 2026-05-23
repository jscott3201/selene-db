//! Questioned edge (`?`) join-tree operator.

use std::collections::BTreeSet;

use selene_core::{EdgeId, NodeId, Value};

use crate::{
    EdgeDirection, EdgeMatch, JoinTree, PatternPlan,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError},
};

use super::{pattern, scan};

pub(crate) fn execute(
    child: &JoinTree,
    edge: &EdgeMatch,
    direction: EdgeDirection,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let child_rows = pattern::walk_join_tree(child, env)?;
    let mut rows = Vec::new();
    let mut state = QuestionedState {
        edge,
        pattern_plan: env.pattern,
        schema: env.schema,
        ctx: env.ctx,
        output: &mut rows,
    };
    for row in child_rows {
        let Some(source) = source_node(edge, env.pattern, env.schema, &row)? else {
            continue;
        };
        maybe_emit_skipped(source, &row, &mut state)?;
        expand_from_source(source, &row, direction, &mut state)?;
    }
    Ok(rows)
}

struct QuestionedState<'a, 'eval, 'ctx, 'g, 'plan, 'out> {
    edge: &'a EdgeMatch,
    pattern_plan: &'a PatternPlan,
    schema: &'a BindingTableSchema,
    ctx: &'a EvalCtx<'eval, 'ctx, 'g, 'plan>,
    output: &'out mut Vec<Binding>,
}

fn maybe_emit_skipped(
    source: NodeId,
    row: &Binding,
    state: &mut QuestionedState<'_, '_, '_, '_, '_, '_>,
) -> Result<(), ExecutorError> {
    if !right_node_matches(state.edge, source, state.ctx) {
        return Ok(());
    }
    let mut values = row.values().to_vec();
    values.resize(state.schema.columns.len(), Value::Null);
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.binding,
        Value::Null,
    )? {
        return Ok(());
    }
    if !pattern::set_hidden_value(
        &mut values,
        state.schema,
        state.edge.hidden_binding,
        Value::Null,
    )? {
        return Ok(());
    }
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.right_binding,
        Value::NodeRef(source),
    )? {
        return Ok(());
    }
    if !pattern::set_hidden_value(
        &mut values,
        state.schema,
        state.edge.right_hidden_binding,
        Value::NodeRef(source),
    )? {
        return Ok(());
    }
    let candidate = Binding::new(values);
    if !predicates_pass(
        &state.edge.right_property_predicates,
        state.pattern_plan,
        &candidate,
        state.schema,
        &Value::NodeRef(source),
        state.ctx,
    )? {
        return Ok(());
    }
    state.output.push(candidate);
    Ok(())
}

fn expand_from_source(
    source: NodeId,
    row: &Binding,
    direction: EdgeDirection,
    state: &mut QuestionedState<'_, '_, '_, '_, '_, '_>,
) -> Result<(), ExecutorError> {
    let mut seen = BTreeSet::new();
    match direction {
        EdgeDirection::Right => {
            if let Some(entry) = state.ctx.tx.snapshot().outgoing_edges(source) {
                for adjacent in entry.iter() {
                    maybe_emit_taken(adjacent.edge_id, adjacent.neighbor, row, state)?;
                }
            }
        }
        EdgeDirection::Left => {
            if let Some(entry) = state.ctx.tx.snapshot().incoming_edges(source) {
                for adjacent in entry.iter() {
                    maybe_emit_taken(adjacent.edge_id, adjacent.neighbor, row, state)?;
                }
            }
        }
        EdgeDirection::Undirected => {
            if let Some(entry) = state.ctx.tx.snapshot().outgoing_edges(source) {
                for adjacent in entry.iter() {
                    if seen.insert(adjacent.edge_id) {
                        maybe_emit_taken(adjacent.edge_id, adjacent.neighbor, row, state)?;
                    }
                }
            }
            if let Some(entry) = state.ctx.tx.snapshot().incoming_edges(source) {
                for adjacent in entry.iter() {
                    if seen.insert(adjacent.edge_id) {
                        maybe_emit_taken(adjacent.edge_id, adjacent.neighbor, row, state)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn maybe_emit_taken(
    edge_id: EdgeId,
    right_node: NodeId,
    row: &Binding,
    state: &mut QuestionedState<'_, '_, '_, '_, '_, '_>,
) -> Result<(), ExecutorError> {
    if !edge_label_matches(state.edge, edge_id, state.ctx)
        || !right_node_matches(state.edge, right_node, state.ctx)
    {
        return Ok(());
    }

    let mut values = row.values().to_vec();
    values.resize(state.schema.columns.len(), Value::Null);
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.binding,
        Value::EdgeRef(edge_id),
    )? {
        return Ok(());
    }
    if !pattern::set_hidden_value(
        &mut values,
        state.schema,
        state.edge.hidden_binding,
        Value::EdgeRef(edge_id),
    )? {
        return Ok(());
    }
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.right_binding,
        Value::NodeRef(right_node),
    )? {
        return Ok(());
    }
    if !pattern::set_hidden_value(
        &mut values,
        state.schema,
        state.edge.right_hidden_binding,
        Value::NodeRef(right_node),
    )? {
        return Ok(());
    }
    let candidate = Binding::new(values);
    if !predicates_pass(
        &state.edge.property_predicates,
        state.pattern_plan,
        &candidate,
        state.schema,
        &Value::EdgeRef(edge_id),
        state.ctx,
    )? || !predicates_pass(
        &state.edge.right_property_predicates,
        state.pattern_plan,
        &candidate,
        state.schema,
        &Value::NodeRef(right_node),
        state.ctx,
    )? {
        return Ok(());
    }
    state.output.push(candidate);
    Ok(())
}

fn source_node(
    edge: &EdgeMatch,
    pattern_plan: &PatternPlan,
    schema: &BindingTableSchema,
    row: &Binding,
) -> Result<Option<NodeId>, ExecutorError> {
    let index = if let Some(binding) = edge.left_binding {
        pattern::binding_index(pattern_plan, schema, binding)
    } else if let Some(hidden) = edge.left_hidden_binding {
        pattern::hidden_index(schema, hidden)
    } else {
        None
    };
    let Some(index) = index else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "questioned source binding column missing",
        });
    };
    match row.get(index).cloned().unwrap_or(Value::Null) {
        Value::NodeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "questioned source binding is not a node",
        }),
    }
}

fn edge_label_matches(edge: &EdgeMatch, edge_id: EdgeId, ctx: &EvalCtx<'_, '_, '_, '_>) -> bool {
    let Some(label_expr) = &edge.label_predicate else {
        return true;
    };
    ctx.tx
        .snapshot()
        .edge_label(edge_id)
        .is_some_and(|label| scan::label_matches_edge(label_expr, *label))
}

fn right_node_matches(edge: &EdgeMatch, node: NodeId, ctx: &EvalCtx<'_, '_, '_, '_>) -> bool {
    let Some(label_expr) = &edge.right_label_predicate else {
        return true;
    };
    ctx.tx
        .snapshot()
        .node_labels(node)
        .is_some_and(|labels| scan::label_matches_node(label_expr, labels))
}

fn predicates_pass(
    predicates: &[crate::FilterPredicate],
    pattern_plan: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    entity: &Value,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    for predicate in predicates {
        if !scan::predicate_passes(predicate, pattern_plan, row, schema, entity, ctx)? {
            return Ok(false);
        }
    }
    Ok(true)
}
