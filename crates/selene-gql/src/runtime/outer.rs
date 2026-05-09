//! Left-outer join-tree operator.

use selene_core::IStr;

use crate::{
    JoinTree,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

pub(crate) fn execute(
    left: &JoinTree,
    right: &JoinTree,
    key: &[IStr],
    env: pattern::WalkContext<'_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let left_rows = pattern::walk_join_tree(left, env)?;
    let mut output = Vec::new();
    for left_row in left_rows {
        let right_env = pattern::WalkContext {
            pattern: env.pattern,
            schema: env.schema,
            seed: Some(&left_row),
            ctx: env.ctx,
        };
        let right_rows = pattern::walk_join_tree(right, right_env)?;
        let mut matched = false;
        for right_row in right_rows {
            if !pattern::rows_match_on_key(&left_row, &right_row, env.schema, key)? {
                continue;
            }
            output.push(pattern::merge_rows(&left_row, &right_row, env.schema));
            matched = true;
        }
        if !matched {
            output.push(left_row);
        }
    }
    Ok(output)
}
