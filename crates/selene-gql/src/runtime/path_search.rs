//! Path-selector wrapper operator.

use rustc_hash::{FxHashMap, FxHashSet};
use selene_core::{NodeId, Value};

use crate::{
    BindingId, HiddenBindingId, HopContributor, JoinTree, PathSelector, TailBinding,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

type EndpointPair = (NodeId, NodeId);

pub(crate) fn execute(
    child: &JoinTree,
    selector: PathSelector,
    source_binding: TailBinding,
    final_binding: TailBinding,
    hop_contributors: &[HopContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let child_rows = pattern::walk_join_tree(child, env)?;
    match selector {
        PathSelector::All => Ok(child_rows),
        PathSelector::Any => select_any(child_rows, source_binding, final_binding, env),
        PathSelector::AllShortest => select_shortest(
            child_rows,
            source_binding,
            final_binding,
            hop_contributors,
            env,
            ShortestMode::All,
        ),
        PathSelector::AnyShortest => select_shortest(
            child_rows,
            source_binding,
            final_binding,
            hop_contributors,
            env,
            ShortestMode::Any,
        ),
    }
}

fn select_any(
    rows: Vec<Binding>,
    source_binding: TailBinding,
    final_binding: TailBinding,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let mut selected = Vec::new();
    let mut seen = FxHashSet::default();
    let mut rows_since_check = 0;
    for row in rows {
        env.ctx
            .tx
            .check_cancellation_stride(&mut rows_since_check, 1)?;
        let Some(pair) = endpoint_pair(&row, source_binding, final_binding, env)? else {
            continue;
        };
        if seen.insert(pair) {
            selected.push(row);
        }
    }
    Ok(selected)
}

#[derive(Clone, Copy)]
enum ShortestMode {
    All,
    Any,
}

fn select_shortest(
    rows: Vec<Binding>,
    source_binding: TailBinding,
    final_binding: TailBinding,
    hop_contributors: &[HopContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
    mode: ShortestMode,
) -> Result<Vec<Binding>, ExecutorError> {
    let mut min_hops = FxHashMap::default();
    let mut rows_since_check = 0;
    for row in &rows {
        env.ctx
            .tx
            .check_cancellation_stride(&mut rows_since_check, 1)?;
        let Some(pair) = endpoint_pair(row, source_binding, final_binding, env)? else {
            continue;
        };
        let hops = hop_count(row, hop_contributors, env)?;
        min_hops
            .entry(pair)
            .and_modify(|current: &mut u32| *current = (*current).min(hops))
            .or_insert(hops);
    }

    let mut selected = Vec::new();
    let mut emitted = FxHashSet::default();
    rows_since_check = 0;
    for row in rows {
        env.ctx
            .tx
            .check_cancellation_stride(&mut rows_since_check, 1)?;
        let Some(pair) = endpoint_pair(&row, source_binding, final_binding, env)? else {
            continue;
        };
        let hops = hop_count(&row, hop_contributors, env)?;
        if min_hops.get(&pair) != Some(&hops) {
            continue;
        }
        match mode {
            ShortestMode::All => selected.push(row),
            ShortestMode::Any => {
                if emitted.insert(pair) {
                    selected.push(row);
                }
            }
        }
    }
    Ok(selected)
}

fn endpoint_pair(
    row: &Binding,
    source_binding: TailBinding,
    final_binding: TailBinding,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Option<EndpointPair>, ExecutorError> {
    let Some(source) = node_value(row, source_binding, env)? else {
        return Ok(None);
    };
    let Some(final_node) = node_value(row, final_binding, env)? else {
        return Ok(None);
    };
    Ok(Some((source, final_node)))
}

fn node_value(
    row: &Binding,
    binding: TailBinding,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Option<NodeId>, ExecutorError> {
    match tail_value(row, binding, env)? {
        Value::NodeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "path-search endpoint binding is not a node",
        }),
    }
}

fn hop_count(
    row: &Binding,
    contributors: &[HopContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<u32, ExecutorError> {
    let mut total = 0u32;
    for contributor in contributors {
        let value = match contributor {
            HopContributor::Fixed(count) => *count,
            HopContributor::GroupNamed(binding) => list_len(binding_value(row, *binding, env)?)?,
            HopContributor::GroupHidden(hidden) => list_len(hidden_value(row, *hidden, env)?)?,
        };
        total = total
            .checked_add(value)
            .ok_or(ExecutorError::ImplementationDefined {
                detail: "path-search hop count overflow",
            })?;
    }
    Ok(total)
}

fn list_len(value: Value) -> Result<u32, ExecutorError> {
    match value {
        Value::List(values) => {
            u32::try_from(values.len()).map_err(|_| ExecutorError::ImplementationDefined {
                detail: "path-search hop list length exceeds u32",
            })
        }
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "path-search hop contributor is not a list",
        }),
    }
}

fn tail_value(
    row: &Binding,
    binding: TailBinding,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match binding {
        TailBinding::Named(binding) => binding_value(row, binding, env),
        TailBinding::Hidden(hidden) => hidden_value(row, hidden, env),
    }
}

fn binding_value(
    row: &Binding,
    binding: BindingId,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let Some(index) = pattern::binding_index(env.pattern, env.schema, binding) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-search binding column missing",
        });
    };
    Ok(row.get(index).cloned().unwrap_or(Value::Null))
}

fn hidden_value(
    row: &Binding,
    hidden: HiddenBindingId,
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let Some(index) = pattern::hidden_index(env.schema, hidden) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "path-search hidden binding column missing",
        });
    };
    Ok(row.get(index).cloned().unwrap_or(Value::Null))
}
