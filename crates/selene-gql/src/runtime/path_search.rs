//! Path-selector wrapper operator.

use rustc_hash::{FxHashMap, FxHashSet};
use selene_core::{NodeId, Value};

use crate::{
    BindingId, HiddenBindingId, HopContributor, JoinTree, PathSelector, TailBinding,
    runtime::{Binding, ExecutorError},
};

use super::{pattern, visited_set};

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
    // Per ISO 39075:2024 §16.6 SR2c the equivalences are:
    //   ALL SHORTEST          == SHORTEST 1 ... GROUP  (count = 1, group = true)
    //   ANY SHORTEST          == SHORTEST 1 ... PATH   (count = 1, group = false)
    //   <counted shortest path>  == SHORTEST N PATHS   (count = N, group = false)
    //   <counted shortest group> == SHORTEST N GROUPS  (count = N, group = true)
    // ALL/ANY (non-shortest) are not length-ranked and keep all / one per pair.
    match selector {
        PathSelector::All => Ok(child_rows),
        PathSelector::Any => select_any(child_rows, source_binding, final_binding, env),
        PathSelector::AllShortest => select_counted(
            child_rows,
            source_binding,
            final_binding,
            hop_contributors,
            env,
            1,
            true,
        ),
        PathSelector::AnyShortest => select_counted(
            child_rows,
            source_binding,
            final_binding,
            hop_contributors,
            env,
            1,
            false,
        ),
        PathSelector::CountedShortest { paths } => select_counted(
            child_rows,
            source_binding,
            final_binding,
            hop_contributors,
            env,
            paths,
            false,
        ),
        PathSelector::CountedShortestGroup { groups } => select_counted(
            child_rows,
            source_binding,
            final_binding,
            hop_contributors,
            env,
            groups,
            true,
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

/// Apply the ISO counted shortest path/group selection to `rows`.
///
/// Implements the shortest-based path-selector family (`ALL SHORTEST`,
/// `ANY SHORTEST`, `SHORTEST N PATHS`, `SHORTEST [N] GROUP[S]`) per ISO/IEC
/// 39075:2024 §22.4 General Rules 12-13. Per §16.6 SR2c every variant is an
/// instance of `(count, group)`:
/// - `group == true` is `SHORTEST N GROUPS` (G020 / `ALL SHORTEST` at N=1) —
///   §22.4 GR13b-ii: keep all bindings whose hop count is among the N smallest
///   *distinct* lengths in the partition.
/// - `group == false` is `SHORTEST N PATHS` (G019 / `ANY SHORTEST` at N=1) —
///   §22.4 GR13b-i: keep the first `min(N, |partition|)` bindings in
///   increasing-hop order; the tie order among equal-length bindings is
///   implementation-dependent (US005), here the stable encounter order.
///
/// **Partitioning (GR12):** two bindings share a partition iff their first and
/// last node bindings are equal — keyed by `endpoint_pair`.
///
/// **Order preservation:** Pass 1 records every valid `(row-index, hops)` in
/// encounter order; Pass 2 re-emits the kept rows in that same original order.
/// This makes `(count=1, group=true)` byte-identical to the legacy `ALL
/// SHORTEST` and `(count=1, group=false)` identical to `ANY SHORTEST`.
///
/// `count` is always `>= 1` at runtime (a literal `0` is rejected at build time
/// with GQLSTATUS 22G0F per §16.6 SR2bii). Defensively, if `count == 0` no row
/// is kept (empty result) rather than panicking.
fn select_counted(
    rows: Vec<Binding>,
    source_binding: TailBinding,
    final_binding: TailBinding,
    hop_contributors: &[HopContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
    count: u32,
    group: bool,
) -> Result<Vec<Binding>, ExecutorError> {
    // Pass 1: bucket valid rows by endpoint partition, recording each row's
    // original index and hop count in encounter order (GR12).
    let mut per_pair: FxHashMap<EndpointPair, Vec<(usize, u32)>> = FxHashMap::default();
    let mut rows_since_check = 0;
    for (index, row) in rows.iter().enumerate() {
        env.ctx
            .tx
            .check_cancellation_stride(&mut rows_since_check, 1)?;
        if !visited_set::trail_allows_hops(row, hop_contributors, env)? {
            continue;
        }
        let Some(pair) = endpoint_pair(row, source_binding, final_binding, env)? else {
            continue;
        };
        let hops = hop_count(row, hop_contributors, env)?;
        per_pair.entry(pair).or_default().push((index, hops));
    }

    // Compute the kept row-index set per partition (GR13b).
    let mut kept: FxHashSet<usize> = FxHashSet::default();
    let limit = count as usize;
    for entries in per_pair.values() {
        if group {
            // §22.4 GR13b-ii: keep bindings in the N smallest DISTINCT lengths.
            let mut distinct: Vec<u32> = entries.iter().map(|(_, hops)| *hops).collect();
            distinct.sort_unstable();
            distinct.dedup();
            distinct.truncate(limit);
            let kept_lengths: FxHashSet<u32> = distinct.into_iter().collect();
            for &(index, hops) in entries {
                if kept_lengths.contains(&hops) {
                    kept.insert(index);
                }
            }
        } else if entries.len() <= limit {
            // §22.4 GR13b-i case 1: |PART| <= N retains the whole partition.
            for &(index, _) in entries {
                kept.insert(index);
            }
        } else {
            // §22.4 GR13b-i case 2: stable-sort by hops ascending (the stable
            // order is the US005 impl-dependent tie-break) and keep the first N.
            let mut ranked = entries.clone();
            ranked.sort_by_key(|(_, hops)| *hops);
            for &(index, _) in ranked.iter().take(limit) {
                kept.insert(index);
            }
        }
    }

    // Pass 2: emit the kept rows in their original encounter order.
    Ok(rows
        .into_iter()
        .enumerate()
        .filter(|(index, _)| kept.contains(index))
        .map(|(_, row)| row)
        .collect())
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
            HopContributor::EdgeNamed(binding) => {
                edge_hop_count(binding_value(row, *binding, env)?)?
            }
            HopContributor::EdgeHidden(hidden) => edge_hop_count(hidden_value(row, *hidden, env)?)?,
            HopContributor::QuestionedNamed(binding) => {
                questioned_hop_count(binding_value(row, *binding, env)?)?
            }
            HopContributor::QuestionedHidden(hidden) => {
                questioned_hop_count(hidden_value(row, *hidden, env)?)?
            }
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

fn edge_hop_count(value: Value) -> Result<u32, ExecutorError> {
    match value {
        Value::EdgeRef(_) => Ok(1),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "path-search fixed hop contributor is not an edge",
        }),
    }
}

fn questioned_hop_count(value: Value) -> Result<u32, ExecutorError> {
    match value {
        Value::Null => Ok(0),
        Value::EdgeRef(_) => Ok(1),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "path-search questioned hop contributor is not an edge or null",
        }),
    }
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
