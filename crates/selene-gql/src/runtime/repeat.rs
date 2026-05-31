//! Bounded variable-length edge repeat operator.

use std::collections::BTreeSet;

use selene_core::{EdgeId, NodeId, Value};
use selene_graph::adjacency::AdjacencyEdge;

use crate::{
    EdgeDirection, JoinTree, PathMode, PatternPlan,
    plan::RepeatEdgeMatch,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError},
};

use super::{pattern, scan, visited_set};

pub(crate) fn execute(
    child: &JoinTree,
    edge: &RepeatEdgeMatch,
    direction: EdgeDirection,
    min: u32,
    max: Option<u32>,
    path_mode: PathMode,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    if path_mode != PathMode::Walk && max.is_some() {
        return Err(ExecutorError::FeatureNotInV1_1 {
            feature: "non-WALK variable-length edge execution",
            span: edge.span,
        });
    }

    let child_rows = pattern::walk_join_tree(child, env)?;
    let mut state = RepeatState {
        edge,
        direction,
        min,
        max,
        path_mode,
        cap: env.ctx.impl_defined_caps().max_quantifier,
        pattern_plan: env.pattern,
        schema: env.schema,
        ctx: env.ctx,
        output: Vec::new(),
        steps_since_check: 0,
    };
    for row in child_rows {
        state
            .ctx
            .tx
            .check_cancellation_stride(&mut state.steps_since_check, 1)?;
        let Some(source) = source_node(edge, env.pattern, env.schema, &row)? else {
            continue;
        };
        let mut path_edges = Vec::new();
        traverse(source, &row, &mut path_edges, 0, &mut state)?;
    }
    Ok(state.output)
}

struct RepeatState<'a, 'eval, 'ctx, 'g, 'plan> {
    edge: &'a RepeatEdgeMatch,
    direction: EdgeDirection,
    min: u32,
    max: Option<u32>,
    path_mode: PathMode,
    cap: u32,
    pattern_plan: &'a PatternPlan,
    schema: &'a BindingTableSchema,
    ctx: &'a EvalCtx<'eval, 'ctx, 'g, 'plan>,
    output: Vec<Binding>,
    steps_since_check: usize,
}

fn traverse(
    current: NodeId,
    row: &Binding,
    path_edges: &mut Vec<EdgeId>,
    depth: u32,
    state: &mut RepeatState<'_, '_, '_, '_, '_>,
) -> Result<(), ExecutorError> {
    if depth >= state.min {
        maybe_emit_path(current, row, path_edges, state)?;
    }
    if state.max == Some(depth) {
        return Ok(());
    }

    for adjacent in adjacent_edges(current, state.direction, state.ctx) {
        state
            .ctx
            .tx
            .check_cancellation_stride(&mut state.steps_since_check, 1)?;
        if !edge_step_matches(adjacent.edge_id, row, path_edges, state)? {
            continue;
        }
        let next_depth = depth.saturating_add(1);
        let step = visited_set::repeat_step(
            visited_set::RepeatStepInput {
                path_mode: state.path_mode,
                source: source_node(state.edge, state.pattern_plan, state.schema, row)?.ok_or(
                    ExecutorError::ImplementationDefined {
                        detail: "repeat source binding column missing",
                    },
                )?,
                target: adjacent.neighbor,
                edge: adjacent.edge_id,
                direction: state.direction,
                path_edges,
                next_depth,
                min: state.min,
            },
            pattern::WalkContext {
                pattern: state.pattern_plan,
                schema: state.schema,
                seed: None,
                ctx: state.ctx,
            },
        )?;
        let Some(step) = step else {
            continue;
        };
        if state.max.is_none() && next_depth > state.cap {
            return Err(ExecutorError::ProgramLimitExceeded {
                detail: "max_quantifier",
                span: state.edge.span,
            });
        }
        path_edges.push(adjacent.edge_id);
        if step.terminal {
            maybe_emit_path(adjacent.neighbor, row, path_edges, state)?;
        } else {
            traverse(adjacent.neighbor, row, path_edges, next_depth, state)?;
        }
        path_edges.pop();
    }
    Ok(())
}

fn maybe_emit_path(
    final_node: NodeId,
    row: &Binding,
    path_edges: &[EdgeId],
    state: &mut RepeatState<'_, '_, '_, '_, '_>,
) -> Result<(), ExecutorError> {
    if !final_node_label_matches(state.edge, final_node, state.ctx) {
        return Ok(());
    }

    let mut values = row.values().to_vec();
    values.resize(state.schema.columns.len(), Value::Null);
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.final_binding,
        Value::NodeRef(final_node),
    )? {
        return Ok(());
    }
    if !pattern::set_hidden_value(
        &mut values,
        state.schema,
        state.edge.final_hidden_binding,
        Value::NodeRef(final_node),
    )? {
        return Ok(());
    }
    let group_value = edge_list_value(path_edges);
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.group_binding,
        group_value.clone(),
    )? {
        return Ok(());
    }
    if !pattern::set_hidden_value(
        &mut values,
        state.schema,
        state.edge.group_hidden_binding,
        group_value,
    )? {
        return Ok(());
    }

    let candidate = Binding::new(values);
    if !predicates_pass(
        &state.edge.final_property_predicates,
        &candidate,
        &Value::NodeRef(final_node),
        state,
    )? {
        return Ok(());
    }
    state.output.push(candidate);
    Ok(())
}

fn edge_step_matches(
    edge_id: EdgeId,
    row: &Binding,
    path_edges: &[EdgeId],
    state: &RepeatState<'_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    if !edge_label_matches(state.edge, edge_id, state.ctx) {
        return Ok(false);
    }
    predicates_pass(
        &state.edge.property_predicates,
        row,
        &Value::EdgeRef(edge_id),
        state,
    )
    .and_then(|passes| {
        if !passes || state.edge.inline_predicates.is_empty() {
            return Ok(passes);
        }
        let candidate;
        let predicate_row = if state.edge.group_binding.is_some() {
            candidate = current_step_row(row, path_edges, edge_id, state)?;
            let Some(candidate) = candidate.as_ref() else {
                return Ok(false);
            };
            candidate
        } else {
            row
        };
        pattern::filter_predicates_pass(
            &state.edge.inline_predicates,
            state.pattern_plan,
            predicate_row,
            state.schema,
            state.ctx,
        )
    })
}

fn current_step_row(
    row: &Binding,
    path_edges: &[EdgeId],
    edge_id: EdgeId,
    state: &RepeatState<'_, '_, '_, '_, '_>,
) -> Result<Option<Binding>, ExecutorError> {
    let mut values = row.values().to_vec();
    values.resize(state.schema.columns.len(), Value::Null);
    let edge_list = path_edges
        .iter()
        .copied()
        .chain(std::iter::once(edge_id))
        .map(Value::EdgeRef)
        .collect();
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.group_binding,
        Value::List(edge_list),
    )? {
        return Ok(None);
    }
    Ok(Some(Binding::new(values)))
}

fn edge_list_value(path_edges: &[EdgeId]) -> Value {
    Value::List(path_edges.iter().copied().map(Value::EdgeRef).collect())
}

fn predicates_pass(
    predicates: &[crate::FilterPredicate],
    row: &Binding,
    entity: &Value,
    state: &RepeatState<'_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    for predicate in predicates {
        if !scan::predicate_passes(
            predicate,
            state.pattern_plan,
            row,
            state.schema,
            entity,
            state.ctx,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn source_node(
    edge: &RepeatEdgeMatch,
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
            detail: "repeat source binding column missing",
        });
    };
    match row.get(index).cloned().unwrap_or(Value::Null) {
        Value::NodeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "repeat source binding is not a node",
        }),
    }
}

fn adjacent_edges(
    node: NodeId,
    direction: EdgeDirection,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<AdjacencyEdge> {
    match direction {
        EdgeDirection::Right => ctx
            .tx
            .snapshot()
            .outgoing_edges(node)
            .map(|entry| entry.iter().copied().collect())
            .unwrap_or_default(),
        EdgeDirection::Left => ctx
            .tx
            .snapshot()
            .incoming_edges(node)
            .map(|entry| entry.iter().copied().collect())
            .unwrap_or_default(),
        EdgeDirection::Undirected => {
            let mut seen = BTreeSet::new();
            let mut edges = Vec::new();
            if let Some(entry) = ctx.tx.snapshot().outgoing_edges(node) {
                for adjacent in entry.iter().copied() {
                    if seen.insert(adjacent.edge_id) {
                        edges.push(adjacent);
                    }
                }
            }
            if let Some(entry) = ctx.tx.snapshot().incoming_edges(node) {
                for adjacent in entry.iter().copied() {
                    if seen.insert(adjacent.edge_id) {
                        edges.push(adjacent);
                    }
                }
            }
            edges
        }
    }
}

fn edge_label_matches(
    edge: &RepeatEdgeMatch,
    edge_id: EdgeId,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> bool {
    let Some(label_expr) = &edge.label_predicate else {
        return true;
    };
    ctx.tx
        .snapshot()
        .edge_label(edge_id)
        .is_some_and(|label| scan::label_matches_edge(label_expr, *label))
}

fn final_node_label_matches(
    edge: &RepeatEdgeMatch,
    node: NodeId,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> bool {
    let Some(label_expr) = &edge.final_label_predicate else {
        return true;
    };
    ctx.tx
        .snapshot()
        .node_labels(node)
        .is_some_and(|labels| scan::label_matches_node(label_expr, labels))
}
