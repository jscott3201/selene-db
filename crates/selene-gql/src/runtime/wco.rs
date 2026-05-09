//! Worst-case optimal marker execution.

use crate::{
    JoinTree,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

pub(crate) fn execute_phase_a(
    intersection: &[JoinTree],
    env: pattern::WalkContext<'_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let [inner] = intersection else {
        return Err(ExecutorError::ImplementationDefined {
            detail: if intersection.is_empty() {
                "WorstCaseOptimal with empty intersection"
            } else {
                "WorstCaseOptimal with multiple intersections"
            },
        });
    };
    pattern::walk_join_tree(inner, env)
}
