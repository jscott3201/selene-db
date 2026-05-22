//! Edge-endpoint helpers for catalog DDL.

use selene_core::{IStr, LabelSet};
use selene_graph::{EdgeEndpointDef, GraphTypeDef};

use crate::{EdgeEndpointSpec, ExecutorError, SourceSpan};

pub(super) fn resolve_endpoints(
    spec: &EdgeEndpointSpec,
    graph_type: &GraphTypeDef,
    span: SourceSpan,
) -> Result<(EdgeEndpointDef, EdgeEndpointDef), ExecutorError> {
    Ok((
        single_endpoint(&spec.from_labels, graph_type, span)?,
        single_endpoint(&spec.to_labels, graph_type, span)?,
    ))
}

fn single_endpoint(
    labels: &[IStr],
    graph_type: &GraphTypeDef,
    span: SourceSpan,
) -> Result<EdgeEndpointDef, ExecutorError> {
    let label_set = labels.iter().copied().collect::<LabelSet>();
    graph_type
        .find_node_type_index(&label_set)
        .map(EdgeEndpointDef::NodeType)
        .ok_or_else(|| ExecutorError::GraphTypeViolation {
            message: format!(
                "edge endpoint references unknown node type label set {}",
                render_endpoint_labels(&label_set)
            ),
            span,
        })
}

fn render_endpoint_labels(labels: &LabelSet) -> String {
    labels
        .iter()
        .map(|label| format!(":{label}"))
        .collect::<Vec<_>>()
        .join(",")
}
