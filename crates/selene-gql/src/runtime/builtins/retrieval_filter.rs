//! Shared row-filter helpers for retrieval candidate producers.

use roaring::RoaringBitmap;
use selene_core::{DbString, NodeId, Value};
use selene_graph::{RowIndex, SeleneGraph};

use super::meta::StaticParameter;
use super::vector_common::{invalid_arg, string_arg};
use crate::procedure_registry::ProcedureError;
use crate::runtime::property_filter_rows::{
    edge_rows_with_property_any, node_rows_with_property_any,
};
use crate::{GqlType, ProcedureDefaultValue, ProcedureParameter};

enum EdgeFilterEndpoint {
    Source,
    Target,
    Both,
}

/// Append the legacy node-property filter parameter pair.
pub(super) fn append_node_filter_parameters(params: &mut Vec<ProcedureParameter>) {
    params.push(
        StaticParameter::new("filter_property", GqlType::String, true)
            .with_description("Indexed scalar node property used to admit matching nodes.")
            .with_default_doc("NULL (no property filter)")
            .with_default(ProcedureDefaultValue::Null)
            .into_parameter(),
    );
    params.push(
        StaticParameter::new(
            "filter_values",
            GqlType::List(Box::new(GqlType::AnyProperty)),
            true,
        )
        .with_description("Indexed scalar values admitted by filter_property.")
        .with_default_doc("NULL (no property filter)")
        .with_default(ProcedureDefaultValue::Null)
        .into_parameter(),
    );
}

/// Append the edge-property endpoint filter parameter group.
pub(super) fn append_edge_filter_parameters(params: &mut Vec<ProcedureParameter>) {
    params.push(
        StaticParameter::new("edge_filter_label", GqlType::String, true)
            .with_description("Indexed edge label used to admit incident nodes.")
            .with_default_doc("NULL (no edge filter)")
            .with_default(ProcedureDefaultValue::Null)
            .into_parameter(),
    );
    params.push(
        StaticParameter::new("edge_filter_property", GqlType::String, true)
            .with_description("Indexed scalar edge property used to admit incident nodes.")
            .with_default_doc("NULL (no edge filter)")
            .with_default(ProcedureDefaultValue::Null)
            .into_parameter(),
    );
    params.push(
        StaticParameter::new(
            "edge_filter_values",
            GqlType::List(Box::new(GqlType::AnyProperty)),
            true,
        )
        .with_description("Indexed scalar values admitted by edge_filter_property.")
        .with_default_doc("NULL (no edge filter)")
        .with_default(ProcedureDefaultValue::Null)
        .into_parameter(),
    );
    params.push(
        StaticParameter::new("edge_filter_endpoint", GqlType::String, true)
            .with_description("Endpoint admitted from matching edges: source, target, or both.")
            .with_default_doc("NULL (no edge filter)")
            .with_default(ProcedureDefaultValue::Null)
            .into_parameter(),
    );
}

/// Resolve optional node- and edge-property filters into a node-row bitmap.
pub(super) fn optional_filter_rows(
    proc_name: &'static str,
    snapshot: &SeleneGraph,
    label: &DbString,
    node_property: &Value,
    node_values: &Value,
    edge_filter: Option<(&Value, &Value, &Value, &Value)>,
) -> Result<Option<RoaringBitmap>, ProcedureError> {
    let mut rows =
        optional_node_filter_rows(proc_name, snapshot, label, node_property, node_values)?;
    if let Some((edge_label, edge_property, edge_values, endpoint)) = edge_filter {
        let edge_rows = optional_edge_filter_rows(
            proc_name,
            snapshot,
            label,
            edge_label,
            edge_property,
            edge_values,
            endpoint,
        )?;
        rows = intersect_optional_rows(rows, edge_rows);
    }
    Ok(rows)
}

/// Convert node-row bitmaps into external node ids without row-id arithmetic.
pub(super) fn node_ids_for_rows(
    proc_name: &'static str,
    snapshot: &SeleneGraph,
    rows: &RoaringBitmap,
) -> Result<Vec<NodeId>, ProcedureError> {
    let mut nodes = Vec::with_capacity(usize::try_from(rows.len()).unwrap_or(usize::MAX));
    for raw_row in rows.iter() {
        let row = RowIndex::new(raw_row);
        let node_id = snapshot
            .node_id_for_row(row)
            .ok_or_else(|| ProcedureError::Internal {
                detail: format!("{proc_name} indexed filter row {raw_row} has no node id"),
            })?;
        nodes.push(node_id);
    }
    Ok(nodes)
}

fn optional_node_filter_rows(
    proc_name: &'static str,
    snapshot: &SeleneGraph,
    label: &DbString,
    property: &Value,
    values: &Value,
) -> Result<Option<RoaringBitmap>, ProcedureError> {
    match (property, values) {
        (Value::Null, Value::Null) => Ok(None),
        (Value::Null, _) | (_, Value::Null) => Err(invalid_arg(format!(
            "{proc_name} filter_property and filter_values must both be NULL or both be supplied"
        ))),
        (_, Value::List(values)) => {
            let property = string_arg(proc_name, property, "filter_property")?;
            node_rows_with_property_any(snapshot, label, &property, values)
                .map(Some)
                .ok_or_else(|| {
                    invalid_arg(format!(
                        "{proc_name} filter_property must name an indexed scalar node property and filter_values must match that index kind"
                    ))
                })
        }
        (_, _) => Err(invalid_arg(format!(
            "{proc_name} filter_values must be a LIST<VALUE> or NULL"
        ))),
    }
}

fn optional_edge_filter_rows(
    proc_name: &'static str,
    snapshot: &SeleneGraph,
    candidate_label: &DbString,
    edge_label: &Value,
    edge_property: &Value,
    edge_values: &Value,
    endpoint: &Value,
) -> Result<Option<RoaringBitmap>, ProcedureError> {
    match (edge_label, edge_property, edge_values, endpoint) {
        (Value::Null, Value::Null, Value::Null, Value::Null) => Ok(None),
        (Value::String(_), Value::String(_), Value::List(values), Value::String(_)) => {
            let label = string_arg(proc_name, edge_label, "edge_filter_label")?;
            let property = string_arg(proc_name, edge_property, "edge_filter_property")?;
            let endpoint = edge_endpoint_arg(proc_name, endpoint)?;
            let edge_rows = edge_rows_with_property_any(snapshot, &label, &property, values)
                .ok_or_else(|| {
                    invalid_arg(format!(
                        "{proc_name} edge_filter_property must name an indexed scalar edge property and edge_filter_values must match that index kind"
                    ))
                })?;
            edge_rows_to_node_rows(proc_name, snapshot, candidate_label, &edge_rows, endpoint)
                .map(Some)
        }
        (_, _, Value::List(_), _) => Err(invalid_arg(format!(
            "{proc_name} edge_filter_label, edge_filter_property, edge_filter_values, and edge_filter_endpoint must all be NULL or all be supplied"
        ))),
        _ => Err(invalid_arg(format!(
            "{proc_name} edge_filter_values must be a LIST<VALUE> and edge_filter_endpoint must be source, target, or both"
        ))),
    }
}

fn edge_rows_to_node_rows(
    proc_name: &'static str,
    snapshot: &SeleneGraph,
    candidate_label: &DbString,
    edge_rows: &RoaringBitmap,
    endpoint: EdgeFilterEndpoint,
) -> Result<RoaringBitmap, ProcedureError> {
    let Some(label_rows) = snapshot.nodes_with_label(candidate_label) else {
        return Ok(RoaringBitmap::new());
    };
    let mut rows = RoaringBitmap::new();
    for raw_edge_row in edge_rows.iter() {
        let edge_id = snapshot
            .edge_id_for_row(RowIndex::new(raw_edge_row))
            .ok_or_else(|| ProcedureError::Internal {
                detail: format!(
                    "{proc_name} indexed edge filter row {raw_edge_row} has no edge id"
                ),
            })?;
        let (source, target) =
            snapshot
                .edge_endpoints(edge_id)
                .ok_or_else(|| ProcedureError::Internal {
                    detail: format!(
                        "{proc_name} indexed edge filter row {raw_edge_row} has no endpoints"
                    ),
                })?;
        match endpoint {
            EdgeFilterEndpoint::Source => insert_node_row(snapshot, label_rows, &mut rows, source),
            EdgeFilterEndpoint::Target => insert_node_row(snapshot, label_rows, &mut rows, target),
            EdgeFilterEndpoint::Both => {
                insert_node_row(snapshot, label_rows, &mut rows, source);
                insert_node_row(snapshot, label_rows, &mut rows, target);
            }
        }
    }
    Ok(rows)
}

fn insert_node_row(
    snapshot: &SeleneGraph,
    label_rows: &RoaringBitmap,
    rows: &mut RoaringBitmap,
    node: NodeId,
) {
    if let Some(row) = snapshot.row_for_node_id(node)
        && label_rows.contains(row.get())
    {
        rows.insert(row.get());
    }
}

fn edge_endpoint_arg(
    proc_name: &'static str,
    value: &Value,
) -> Result<EdgeFilterEndpoint, ProcedureError> {
    let raw = string_arg(proc_name, value, "edge_filter_endpoint")?;
    match raw.as_str() {
        "source" | "from" | "outgoing" | "out" => Ok(EdgeFilterEndpoint::Source),
        "target" | "to" | "incoming" | "in" => Ok(EdgeFilterEndpoint::Target),
        "both" | "either" | "any" => Ok(EdgeFilterEndpoint::Both),
        other => Err(invalid_arg(format!(
            "{proc_name} unknown edge_filter_endpoint '{other}'; expected source, target, or both"
        ))),
    }
}

fn intersect_optional_rows(
    left: Option<RoaringBitmap>,
    right: Option<RoaringBitmap>,
) -> Option<RoaringBitmap> {
    match (left, right) {
        (Some(mut left), Some(right)) => {
            left &= right;
            Some(left)
        }
        (Some(rows), None) | (None, Some(rows)) => Some(rows),
        (None, None) => None,
    }
}
