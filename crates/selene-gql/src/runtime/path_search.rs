//! Path-selector wrapper operator.

use crate::{
    HopContributor, JoinTree, PathSelector, TailBinding,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

pub(crate) fn execute(
    _child: &JoinTree,
    _selector: PathSelector,
    _source_binding: TailBinding,
    _final_binding: TailBinding,
    _hop_contributors: &[HopContributor],
    _env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    Err(ExecutorError::FeatureNotInV1_1 {
        feature: "MATCH path selector execution",
        span: crate::SourceSpan::default(),
    })
}
