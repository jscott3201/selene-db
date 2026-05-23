//! Path-mode wrapper operator.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    JoinTree, PathContributor, PathMode,
    runtime::{Binding, ExecutorError},
};

use super::{pattern, visited_set};

pub(crate) fn execute(
    child: &JoinTree,
    path_mode: PathMode,
    path_contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let child_rows = pattern::walk_join_tree(child, env)?;
    let mut selected = Vec::new();
    let mut rows_since_check = 0;
    for row in child_rows {
        env.ctx
            .tx
            .check_cancellation_stride(&mut rows_since_check, 1)?;
        if validates(&row, path_mode, path_contributors, env)? {
            selected.push(row);
        }
    }
    Ok(selected)
}

fn validates(
    row: &Binding,
    path_mode: PathMode,
    path_contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    match path_mode {
        PathMode::Walk => Ok(true),
        PathMode::Trail => visited_set::trail_allows_path(row, path_contributors, env),
        PathMode::Acyclic => validate_acyclic(row, path_contributors, env),
        PathMode::Simple => validate_simple(row, path_contributors, env),
    }
}

fn validate_acyclic(
    row: &Binding,
    path_contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    let mut seen = FxHashSet::default();
    for node in visited_set::collect_path_nodes(row, path_contributors, env)? {
        if !seen.insert(node) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_simple(
    row: &Binding,
    path_contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    let nodes = visited_set::collect_path_nodes(row, path_contributors, env)?;
    let mut counts = FxHashMap::default();
    for node in &nodes {
        *counts.entry(*node).or_insert(0usize) += 1;
    }
    let repeated = counts
        .iter()
        .filter_map(|(node, count)| (*count > 1).then_some((*node, *count)))
        .collect::<Vec<_>>();
    match repeated.as_slice() {
        [] => Ok(true),
        [(node, 2)]
            if nodes.first() == Some(node)
                && nodes.last() == Some(node)
                && nodes.iter().filter(|candidate| *candidate == node).count() == 2 =>
        {
            Ok(true)
        }
        _ => Ok(false),
    }
}
