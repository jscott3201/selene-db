//! Edge-endpoint helpers for catalog DDL.

use selene_core::{IStr, LabelSet};
use selene_graph::GraphTypeDef;

use crate::{EdgeEndpointSpec, ExecutorError, SourceSpan};

pub(super) fn resolve_endpoints(
    spec: &EdgeEndpointSpec,
    graph_type: &GraphTypeDef,
    span: SourceSpan,
) -> Result<(u32, u32), ExecutorError> {
    Ok((
        single_endpoint(&spec.from_labels, graph_type, span)?,
        single_endpoint(&spec.to_labels, graph_type, span)?,
    ))
}

fn single_endpoint(
    labels: &[IStr],
    graph_type: &GraphTypeDef,
    span: SourceSpan,
) -> Result<u32, ExecutorError> {
    let [label] = labels else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "multi-label edge endpoint not supported (Phase A: single label per endpoint)",
        });
    };
    graph_type
        .find_node_type_index(&LabelSet::single(*label))
        .ok_or_else(|| ExecutorError::GraphTypeViolation {
            message: format!("edge endpoint references unknown node type label {label}"),
            span,
        })
}
