//! Shared path visited-set helpers.

use rustc_hash::FxHashSet;
use selene_core::{EdgeId, NodeId, Value};

use crate::{
    BindingId, EdgeDirection, HiddenBindingId, HopContributor, PathContributor, TailBinding,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

pub(crate) fn trail_allows_hops(
    row: &Binding,
    contributors: &[HopContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    let mut seen = FxHashSet::default();
    for contributor in contributors {
        match contributor {
            HopContributor::Fixed(0) => {}
            HopContributor::Fixed(_) => {
                return Err(ExecutorError::ImplementationDefined {
                    detail: "path-search trail contributor lacks edge identity",
                });
            }
            HopContributor::EdgeNamed(binding) => {
                if !insert_edge_value(row_binding_value(row, *binding, env)?, &mut seen)? {
                    return Ok(false);
                }
            }
            HopContributor::EdgeHidden(hidden) => {
                if !insert_edge_value(row_hidden_value(row, *hidden, env)?, &mut seen)? {
                    return Ok(false);
                }
            }
            HopContributor::GroupNamed(binding) => {
                if !insert_edge_list(row_binding_value(row, *binding, env)?, &mut seen)? {
                    return Ok(false);
                }
            }
            HopContributor::GroupHidden(hidden) => {
                if !insert_edge_list(row_hidden_value(row, *hidden, env)?, &mut seen)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

pub(crate) fn trail_allows_path(
    row: &Binding,
    contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    let mut seen = FxHashSet::default();
    for edge in collect_path_edges(row, contributors, env)? {
        if !seen.insert(edge) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn collect_path_nodes(
    row: &Binding,
    contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<NodeId>, ExecutorError> {
    let mut nodes = Vec::new();
    for contributor in contributors {
        match *contributor {
            PathContributor::Node(binding) => {
                if let Some(node) = node_value(row_tail_value(row, binding, env)?)? {
                    nodes.push(node);
                }
            }
            PathContributor::EdgeNamed(_) | PathContributor::EdgeHidden(_) => {}
            PathContributor::EdgeGroupNamed {
                binding,
                source,
                direction,
            } => {
                let edges = edge_list(row_binding_value(row, binding, env)?)?;
                append_group_nodes(row, source, direction, &edges, env, &mut nodes)?;
            }
            PathContributor::EdgeGroupHidden {
                hidden,
                source,
                direction,
            } => {
                let edges = edge_list(row_hidden_value(row, hidden, env)?)?;
                append_group_nodes(row, source, direction, &edges, env, &mut nodes)?;
            }
        }
    }
    Ok(nodes)
}

fn collect_path_edges(
    row: &Binding,
    contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<EdgeId>, ExecutorError> {
    let mut edges = Vec::new();
    for contributor in contributors {
        match *contributor {
            PathContributor::Node(_) => {}
            PathContributor::EdgeNamed(binding) => {
                edges.push(edge_value(row_binding_value(row, binding, env)?)?);
            }
            PathContributor::EdgeHidden(hidden) => {
                edges.push(edge_value(row_hidden_value(row, hidden, env)?)?);
            }
            PathContributor::EdgeGroupNamed { binding, .. } => {
                edges.extend(edge_list(row_binding_value(row, binding, env)?)?);
            }
            PathContributor::EdgeGroupHidden { hidden, .. } => {
                edges.extend(edge_list(row_hidden_value(row, hidden, env)?)?);
            }
        }
    }
    Ok(edges)
}

fn append_group_nodes(
    row: &Binding,
    source: TailBinding,
    direction: EdgeDirection,
    edges: &[EdgeId],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
    nodes: &mut Vec<NodeId>,
) -> Result<(), ExecutorError> {
    if edges.is_empty() {
        return Ok(());
    }
    let Some(mut current) = node_value(row_tail_value(row, source, env)?)? else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-mode quantified edge source is null",
        });
    };
    if nodes.last().copied() != Some(current) {
        nodes.push(current);
    }
    for edge in edges {
        let next = next_node(*edge, current, direction, env)?;
        nodes.push(next);
        current = next;
    }
    Ok(())
}

fn next_node(
    edge: EdgeId,
    current: NodeId,
    direction: EdgeDirection,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<NodeId, ExecutorError> {
    let Some((source, target)) = env.ctx.tx.snapshot().edge_endpoints(edge) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-mode contributor references missing edge",
        });
    };
    match direction {
        EdgeDirection::Right if source == current => Ok(target),
        EdgeDirection::Left if target == current => Ok(source),
        EdgeDirection::Undirected if source == current => Ok(target),
        EdgeDirection::Undirected if target == current => Ok(source),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "path-mode edge endpoints are inconsistent with path direction",
        }),
    }
}

fn insert_edge_list(value: Value, seen: &mut FxHashSet<EdgeId>) -> Result<bool, ExecutorError> {
    for edge in edge_list(value)? {
        if !seen.insert(edge) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn insert_edge_value(value: Value, seen: &mut FxHashSet<EdgeId>) -> Result<bool, ExecutorError> {
    Ok(seen.insert(edge_value(value)?))
}

fn edge_list(value: Value) -> Result<Vec<EdgeId>, ExecutorError> {
    let Value::List(values) = value else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-mode edge group contributor is not a list",
        });
    };
    values.into_iter().map(edge_value).collect()
}

fn edge_value(value: Value) -> Result<EdgeId, ExecutorError> {
    let Value::EdgeRef(edge) = value else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-mode edge contributor is not an edge",
        });
    };
    Ok(edge)
}

fn node_value(value: Value) -> Result<Option<NodeId>, ExecutorError> {
    match value {
        Value::NodeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "path-mode node contributor is not a node",
        }),
    }
}

fn row_tail_value(
    row: &Binding,
    binding: TailBinding,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match binding {
        TailBinding::Named(binding) => row_binding_value(row, binding, env),
        TailBinding::Hidden(hidden) => row_hidden_value(row, hidden, env),
    }
}

fn row_binding_value(
    row: &Binding,
    binding: BindingId,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let Some(index) = pattern::binding_index(env.pattern, env.schema, binding) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-mode binding column missing",
        });
    };
    Ok(row.get(index).cloned().unwrap_or(Value::Null))
}

fn row_hidden_value(
    row: &Binding,
    hidden: HiddenBindingId,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let Some(index) = pattern::hidden_index(env.schema, hidden) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-mode hidden binding column missing",
        });
    };
    Ok(row.get(index).cloned().unwrap_or(Value::Null))
}
