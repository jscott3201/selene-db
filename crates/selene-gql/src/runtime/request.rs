//! Facade request input and parameter preflight.

use std::collections::BTreeMap;

use selene_core::{DbString, EdgeDirection, Path, Record, Value};
use selene_graph::SeleneGraph;

use crate::{GqlType, SourceSpan, analyze::ParameterUse};

use super::{ExecutorError, parameter_type};

/// Typed parameter input for one facade request.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct RequestParameter {
    declared_type: GqlType,
    value: Value,
}

impl RequestParameter {
    /// Construct one typed request parameter.
    #[must_use]
    pub const fn new(declared_type: GqlType, value: Value) -> Self {
        Self {
            declared_type,
            value,
        }
    }

    pub(crate) const fn declared_type(&self) -> &GqlType {
        &self.declared_type
    }

    pub(crate) const fn value(&self) -> &Value {
        &self.value
    }
}

/// Explicit lower-runtime input for one facade request.
#[doc(hidden)]
pub struct RequestExecutionInput {
    pub(crate) parameters: BTreeMap<DbString, RequestParameter>,
    pub(crate) timestamp: jiff::Timestamp,
    pub(crate) time_zone: jiff::tz::TimeZone,
}

impl RequestExecutionInput {
    /// Construct lower request input from a facade-owned immutable snapshot.
    #[must_use]
    pub fn new(
        parameters: BTreeMap<DbString, RequestParameter>,
        timestamp: jiff::Timestamp,
        time_zone: jiff::tz::TimeZone,
    ) -> Self {
        Self {
            parameters,
            timestamp,
            time_zone,
        }
    }
}

pub(super) fn validate(
    request: &RequestExecutionInput,
    uses: &[ParameterUse],
    graph: &SeleneGraph,
) -> Result<(), ExecutorError> {
    for (name, parameter) in &request.parameters {
        parameter_type::validate_declared_type(
            name.clone(),
            parameter.value(),
            parameter.declared_type(),
            SourceSpan::default(),
        )?;
        validate_value_references(name, parameter.value(), graph, SourceSpan::default())?;
    }
    for parameter_use in uses {
        let parameter = request.parameters.get(&parameter_use.name).ok_or_else(|| {
            ExecutorError::UnboundParameter {
                name: parameter_use.name.clone(),
                span: parameter_use.span,
            }
        })?;
        if let Some(declared_type) = &parameter_use.declared_type {
            parameter_type::validate_declared_type(
                parameter_use.name.clone(),
                parameter.value(),
                declared_type,
                parameter_use.span,
            )?;
        }
    }
    Ok(())
}

pub(super) fn validate_references(
    request: &RequestExecutionInput,
    graph: &SeleneGraph,
) -> Result<(), ExecutorError> {
    for (name, parameter) in &request.parameters {
        validate_value_references(name, parameter.value(), graph, SourceSpan::default())?;
    }
    Ok(())
}

fn validate_value_references(
    name: &DbString,
    value: &Value,
    graph: &SeleneGraph,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::GraphRef(graph_id) if *graph_id != graph.graph_id() => {
                return Err(invalid_reference(
                    name,
                    "graph reference selects another graph",
                    span,
                ));
            }
            Value::NodeRef(node_id) if !graph.is_node_alive(*node_id) => {
                return Err(invalid_reference(name, "node reference is not alive", span));
            }
            Value::EdgeRef(edge_id) if !graph.is_edge_alive(*edge_id) => {
                return Err(invalid_reference(name, "edge reference is not alive", span));
            }
            Value::Path(path) => validate_path(name, path, graph, span)?,
            Value::TableRef(_) => {
                return Err(invalid_reference(
                    name,
                    "table parameters are pending M03-PR03",
                    span,
                ));
            }
            Value::List(values) => pending.extend(values),
            Value::Record(record) => {
                if let Record::Open(fields) = record.as_ref() {
                    pending.extend(fields.iter().map(|(_, value)| value));
                }
            }
            Value::RecordTyped(record) => pending.extend(record.values.iter().flatten()),
            _ => {}
        }
    }
    Ok(())
}

fn validate_path(
    name: &DbString,
    path: &Path,
    graph: &SeleneGraph,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    if path.graph != graph.graph_id() {
        return Err(invalid_reference(name, "path selects another graph", span));
    }
    if !graph.is_node_alive(path.start) {
        return Err(invalid_reference(
            name,
            "path start node is not alive",
            span,
        ));
    }
    let mut current = path.start;
    for segment in &path.segments {
        if !graph.is_node_alive(segment.node) || !graph.is_edge_alive(segment.edge) {
            return Err(invalid_reference(
                name,
                "path contains a stale element",
                span,
            ));
        }
        let Some((source, target)) = graph.edge_endpoints(segment.edge) else {
            return Err(invalid_reference(
                name,
                "path edge has no live endpoints",
                span,
            ));
        };
        let connected = match segment.direction {
            EdgeDirection::Outgoing => source == current && target == segment.node,
            EdgeDirection::Incoming => target == current && source == segment.node,
            EdgeDirection::Undirected => {
                (source == current && target == segment.node)
                    || (target == current && source == segment.node)
            }
        };
        if !connected {
            return Err(invalid_reference(
                name,
                "path traversal is not connected",
                span,
            ));
        }
        current = segment.node;
    }
    Ok(())
}

fn invalid_reference(name: &DbString, detail: &str, span: SourceSpan) -> ExecutorError {
    ExecutorError::InvalidReference {
        name: format!("parameter ${name}: {detail}"),
        span,
    }
}
