//! Expand join-tree operator.

use std::collections::BTreeSet;

use selene_core::{EdgeId, NodeId, Value};

use crate::{
    EdgeDirection, EdgeMatch, JoinTree, PatternPlan, ScanKind,
    runtime::{Binding, BindingTableSchema, ExecutorError, TxContext},
};

use super::{pattern, scan};

pub(crate) fn execute(
    child: &JoinTree,
    edge: &EdgeMatch,
    direction: EdgeDirection,
    env: pattern::WalkContext<'_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    if edge.left_binding.is_none() {
        return execute_anonymous_source(child, edge, direction, env);
    }

    let child_rows = pattern::walk_join_tree(child, env)?;
    let mut rows = Vec::new();
    let mut state = ExpandState {
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
        expand_from_source(source, &row, direction, &mut state)?;
    }
    Ok(rows)
}

fn execute_anonymous_source(
    child: &JoinTree,
    edge: &EdgeMatch,
    direction: EdgeDirection,
    env: pattern::WalkContext<'_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let JoinTree::Scan(scan_node) = child else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "expand source binding missing",
        });
    };
    if scan_node.kind != ScanKind::Node || scan_node.binding.is_some() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "expand source binding missing",
        });
    }

    let mut rows = Vec::new();
    let mut state = ExpandState {
        edge,
        pattern_plan: env.pattern,
        schema: env.schema,
        ctx: env.ctx,
        output: &mut rows,
    };
    for (entity, row) in scan::scan_entities(scan_node, env.pattern, env.schema, env.seed, env.ctx)?
    {
        let Value::NodeRef(source) = entity else {
            continue;
        };
        expand_from_source(source, &row, direction, &mut state)?;
    }
    Ok(rows)
}

struct ExpandState<'a, 'ctx, 'out> {
    edge: &'a EdgeMatch,
    pattern_plan: &'a PatternPlan,
    schema: &'a BindingTableSchema,
    ctx: &'a TxContext<'ctx>,
    output: &'out mut Vec<Binding>,
}

fn source_node(
    edge: &EdgeMatch,
    pattern_plan: &PatternPlan,
    schema: &BindingTableSchema,
    row: &Binding,
) -> Result<Option<NodeId>, ExecutorError> {
    let Some(binding) = edge.left_binding else {
        return Ok(None);
    };
    let Some(index) = pattern::binding_index(pattern_plan, schema, binding) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "expand source binding column missing",
        });
    };
    match row.get(index).cloned().unwrap_or(Value::Null) {
        Value::NodeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "expand source binding is not a node",
        }),
    }
}

fn expand_from_source(
    source: NodeId,
    row: &Binding,
    direction: EdgeDirection,
    state: &mut ExpandState<'_, '_, '_>,
) -> Result<(), ExecutorError> {
    let mut seen = BTreeSet::new();
    match direction {
        EdgeDirection::Right => {
            if let Some(entry) = state.ctx.snapshot().outgoing_edges(source) {
                for adjacent in entry.iter() {
                    maybe_emit(adjacent.edge_id, adjacent.neighbor, row, state)?;
                }
            }
        }
        EdgeDirection::Left => {
            if let Some(entry) = state.ctx.snapshot().incoming_edges(source) {
                for adjacent in entry.iter() {
                    maybe_emit(adjacent.edge_id, adjacent.neighbor, row, state)?;
                }
            }
        }
        EdgeDirection::Undirected => {
            if let Some(entry) = state.ctx.snapshot().outgoing_edges(source) {
                for adjacent in entry.iter() {
                    if seen.insert(adjacent.edge_id) {
                        maybe_emit(adjacent.edge_id, adjacent.neighbor, row, state)?;
                    }
                }
            }
            if let Some(entry) = state.ctx.snapshot().incoming_edges(source) {
                for adjacent in entry.iter() {
                    if seen.insert(adjacent.edge_id) {
                        maybe_emit(adjacent.edge_id, adjacent.neighbor, row, state)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn maybe_emit(
    edge_id: EdgeId,
    right_node: NodeId,
    row: &Binding,
    state: &mut ExpandState<'_, '_, '_>,
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
    if !pattern::set_binding_value(
        &mut values,
        state.pattern_plan,
        state.schema,
        state.edge.right_binding,
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
    )? {
        return Ok(());
    }
    if !predicates_pass(
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

fn edge_label_matches(edge: &EdgeMatch, edge_id: EdgeId, ctx: &TxContext<'_>) -> bool {
    let Some(label_expr) = &edge.label_predicate else {
        return true;
    };
    ctx.snapshot()
        .edge_label(edge_id)
        .is_some_and(|label| scan::label_matches_edge(label_expr, *label))
}

fn right_node_matches(edge: &EdgeMatch, node: NodeId, ctx: &TxContext<'_>) -> bool {
    let Some(label_expr) = &edge.right_label_predicate else {
        return true;
    };
    ctx.snapshot()
        .node_labels(node)
        .is_some_and(|labels| scan::label_matches_node(label_expr, labels))
}

fn predicates_pass(
    predicates: &[crate::FilterPredicate],
    pattern_plan: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    entity: &Value,
    ctx: &TxContext<'_>,
) -> Result<bool, ExecutorError> {
    for predicate in predicates {
        if !scan::predicate_passes(predicate, pattern_plan, row, schema, entity, ctx)? {
            return Ok(false);
        }
    }
    Ok(true)
}
