//! Hash-join join-tree operator.

use selene_core::{IStr, Value};

use crate::{
    BuildSide, JoinTree,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

pub(crate) fn execute(
    left: &JoinTree,
    right: &JoinTree,
    key: &[IStr],
    build_side: BuildSide,
    env: pattern::WalkContext<'_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    match build_side {
        BuildSide::Left => execute_ordered(left, right, key, env, true),
        BuildSide::Right => execute_ordered(right, left, key, env, false),
    }
}

fn execute_ordered(
    build_tree: &JoinTree,
    probe_tree: &JoinTree,
    key: &[IStr],
    env: pattern::WalkContext<'_, '_, '_>,
    build_is_left: bool,
) -> Result<Vec<Binding>, ExecutorError> {
    let build_rows = pattern::walk_join_tree(build_tree, env)?;
    let mut build_entries = Vec::new();
    for row in build_rows {
        if let Some(key_values) = pattern::key_values(&row, env.schema, key)? {
            build_entries.push((key_values, row));
        }
    }

    let probe_rows = pattern::walk_join_tree(probe_tree, env)?;
    let mut rows = Vec::new();
    for probe in probe_rows {
        let Some(probe_key) = pattern::key_values(&probe, env.schema, key)? else {
            continue;
        };
        for (build_key, build) in &build_entries {
            if !keys_match(build_key, &probe_key) {
                continue;
            }
            let row = if build_is_left {
                pattern::merge_rows(build, &probe, env.schema)
            } else {
                pattern::merge_rows(&probe, build, env.schema)
            };
            rows.push(row);
        }
    }
    Ok(rows)
}

fn keys_match(build_key: &[Value], probe_key: &[Value]) -> bool {
    pattern::key_values_equal(build_key, probe_key)
}
