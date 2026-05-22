//! Variable-length edge repeat operator.

use crate::{
    EdgeDirection, JoinTree, PathMode, PathSelector,
    plan::RepeatEdgeMatch,
    runtime::{Binding, ExecutorError},
};

use super::pattern;

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    _child: &JoinTree,
    edge: &RepeatEdgeMatch,
    _direction: EdgeDirection,
    _min: u32,
    _max: Option<u32>,
    _path_mode: PathMode,
    _selector: Option<PathSelector>,
    _env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    Err(ExecutorError::FeatureNotInV1_1 {
        feature: "bounded variable-length edge runtime",
        span: edge.span,
    })
}
