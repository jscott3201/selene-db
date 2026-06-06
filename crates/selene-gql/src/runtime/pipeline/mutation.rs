//! Mutation pipeline operator.

use std::collections::BTreeSet;

use smallvec::SmallVec;

use selene_core::{
    DbString, EdgeId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value,
};

use crate::{
    BindingTableColumn, BindingTableSchema, DeleteMode, DeleteTargetPlan, EdgeDirection,
    ElementKind, InsertEndpointRef, LabelExpr, MutationOp, ProjectExpr, PropertyInit, SourceSpan,
    SubqueryRegistry,
    analyze::ExprIdLookup,
    runtime::{
        Binding, BindingTable, DataExceptionSubclass, EvalCtx, ExecutorError, TxContext, evaluator,
    },
};

#[derive(Clone, Copy)]
enum PropertyMapTarget {
    Node,
    Edge,
}

pub(super) fn execute(
    op: &MutationOp,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
    expr_ids: &ExprIdLookup,
    subqueries: &SubqueryRegistry,
) -> Result<BindingTable, ExecutorError> {
    {
        let _mutator = ctx.mutator()?;
    }
    match op {
        MutationOp::InsertNode {
            site_id,
            label_expr,
            property_inits,
            output_column_index,
            output_column,
            span,
            ..
        } => execute_insert_node(
            *site_id,
            label_expr.as_ref(),
            property_inits,
            *output_column_index,
            output_column.as_ref(),
            *span,
            table,
            ctx,
            expr_ids,
            subqueries,
        ),
        MutationOp::InsertEdge {
            label_expr,
            left,
            right,
            direction,
            property_inits,
            output_column_index,
            output_column,
            span,
            ..
        } => execute_insert_edge(
            label_expr.as_ref(),
            *left,
            *right,
            *direction,
            property_inits,
            *output_column_index,
            output_column.as_ref(),
            *span,
            table,
            ctx,
            expr_ids,
            subqueries,
        ),
        MutationOp::SetProperty {
            element,
            target_column_index,
            key,
            value,
            span,
            ..
        } => execute_set_property(
            *element,
            *target_column_index,
            key.clone(),
            value,
            *span,
            table,
            ctx,
            expr_ids,
            subqueries,
        ),
        MutationOp::SetLabel {
            element,
            target_column_index,
            label,
            span,
            ..
        } => execute_set_label(
            *element,
            *target_column_index,
            label.clone(),
            *span,
            table,
            ctx,
        ),
        MutationOp::RemoveProperty {
            element,
            target_column_index,
            key,
            span,
            ..
        } => execute_remove_property(
            *element,
            *target_column_index,
            key.clone(),
            *span,
            table,
            ctx,
        ),
        MutationOp::RemoveLabel {
            element,
            target_column_index,
            label,
            span,
            ..
        } => execute_remove_label(
            *element,
            *target_column_index,
            label.clone(),
            *span,
            table,
            ctx,
        ),
        MutationOp::DeleteTargets {
            targets,
            mode,
            span,
            ..
        } => execute_delete_targets(targets, *mode, *span, table, ctx),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_insert_node(
    site_id: crate::InsertSiteId,
    label_expr: Option<&LabelExpr>,
    property_inits: &[PropertyInit],
    output_column_index: Option<u32>,
    output_column: Option<&BindingTableColumn>,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
    expr_ids: &ExprIdLookup,
    subqueries: &SubqueryRegistry,
) -> Result<BindingTable, ExecutorError> {
    let labels = node_labels(label_expr, span)?;
    let (mut schema, rows) = table.into_parts();
    let input_schema = schema.clone();
    extend_schema(&mut schema, output_column_index, output_column)?;
    let mut output = Vec::with_capacity(rows.len());
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let props = {
            let eval_ctx = EvalCtx {
                tx: ctx,
                expr_ids,
                subqueries,
            };
            property_map(
                property_inits,
                PropertyMapTarget::Node,
                &row,
                &input_schema,
                &eval_ctx,
            )?
        };
        let node_id = {
            let mut mutator = ctx.mutator()?;
            mutator
                .create_node(labels.clone(), props)
                .map_err(|source| graph_mutation(source, span))?
        };
        let mut values = row.values().to_vec();
        if let Some(index) = output_column_index {
            set_output_value(&mut values, index, Value::NodeRef(node_id));
        }
        let mut insert_sites = row.insert_sites().iter().copied().collect::<SmallVec<_>>();
        record_insert_site(&mut insert_sites, site_id, node_id);
        output.push(Binding::with_insert_sites(values, insert_sites));
    }
    Ok(BindingTable::new(schema, output))
}

#[allow(clippy::too_many_arguments)]
fn execute_insert_edge(
    label_expr: Option<&LabelExpr>,
    left: InsertEndpointRef,
    right: InsertEndpointRef,
    direction: EdgeDirection,
    property_inits: &[PropertyInit],
    output_column_index: Option<u32>,
    output_column: Option<&BindingTableColumn>,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
    expr_ids: &ExprIdLookup,
    subqueries: &SubqueryRegistry,
) -> Result<BindingTable, ExecutorError> {
    let label = edge_label(label_expr, span)?;
    let (mut schema, rows) = table.into_parts();
    let input_schema = schema.clone();
    extend_schema(&mut schema, output_column_index, output_column)?;
    let mut output = Vec::with_capacity(rows.len());
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let left = endpoint_node(&row, left, span)?;
        let right = endpoint_node(&row, right, span)?;
        let (source, target) = edge_endpoints(left, right, direction, span)?;
        let props = {
            let eval_ctx = EvalCtx {
                tx: ctx,
                expr_ids,
                subqueries,
            };
            property_map(
                property_inits,
                PropertyMapTarget::Edge,
                &row,
                &input_schema,
                &eval_ctx,
            )?
        };
        let edge_id = {
            let mut mutator = ctx.mutator()?;
            mutator
                .create_edge(label.clone(), source, target, props)
                .map_err(|source| graph_mutation(source, span))?
        };
        let mut values = row.values().to_vec();
        if let Some(index) = output_column_index {
            set_output_value(&mut values, index, Value::EdgeRef(edge_id));
        }
        output.push(Binding::with_insert_sites(
            values,
            row.insert_sites().iter().copied().collect(),
        ));
    }
    Ok(BindingTable::new(schema, output))
}

#[allow(clippy::too_many_arguments)]
fn execute_set_property(
    element: ElementKind,
    target_column_index: u32,
    key: DbString,
    value: &ProjectExpr,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
    expr_ids: &ExprIdLookup,
    subqueries: &SubqueryRegistry,
) -> Result<BindingTable, ExecutorError> {
    let (schema, rows) = table.into_parts();
    let mut rows_since_check = 0;
    for row in &rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let value = {
            let eval_ctx = EvalCtx {
                tx: ctx,
                expr_ids,
                subqueries,
            };
            evaluator::evaluate(&value.expr, row, &schema, &eval_ctx)?
        };
        let diff = property_diff([(key.clone(), value)], [], span)?;
        match element {
            ElementKind::Node => {
                if let Some(id) = target_node(row, target_column_index, span)? {
                    ctx.mutator()?
                        .update_node(id, label_diff([], [], span)?, diff)
                        .map_err(|source| graph_mutation(source, span))?;
                }
            }
            ElementKind::Edge => {
                if let Some(id) = target_edge(row, target_column_index, span)? {
                    ctx.mutator()?
                        .update_edge(id, diff)
                        .map_err(|source| graph_mutation(source, span))?;
                }
            }
            ElementKind::Path | ElementKind::Alias => {
                return Err(unsupported_target(element));
            }
        }
    }
    Ok(BindingTable::new(schema, rows))
}

fn execute_set_label(
    element: ElementKind,
    target_column_index: u32,
    label: DbString,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let diff = label_diff([label], [], span)?;
    let mut rows_since_check = 0;
    for row in table.rows() {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match element {
            ElementKind::Node => {
                if let Some(id) = target_node(row, target_column_index, span)? {
                    ctx.mutator()?
                        .update_node(id, diff.clone(), property_diff([], [], span)?)
                        .map_err(|source| graph_mutation(source, span))?;
                }
            }
            ElementKind::Edge | ElementKind::Path | ElementKind::Alias => {
                return Err(unsupported_target(element));
            }
        }
    }
    Ok(table)
}

fn execute_remove_property(
    element: ElementKind,
    target_column_index: u32,
    key: DbString,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let mut rows_since_check = 0;
    for row in table.rows() {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match element {
            ElementKind::Node => {
                if let Some(id) = target_node(row, target_column_index, span)? {
                    ctx.mutator()?
                        .remove_node_property(id, key.clone())
                        .map_err(|source| graph_mutation(source, span))?;
                }
            }
            ElementKind::Edge => {
                if let Some(id) = target_edge(row, target_column_index, span)? {
                    ctx.mutator()?
                        .remove_edge_property(id, key.clone())
                        .map_err(|source| graph_mutation(source, span))?;
                }
            }
            ElementKind::Path | ElementKind::Alias => {
                return Err(unsupported_target(element));
            }
        }
    }
    Ok(table)
}

fn execute_remove_label(
    element: ElementKind,
    target_column_index: u32,
    label: DbString,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let mut rows_since_check = 0;
    for row in table.rows() {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match element {
            ElementKind::Node => {
                if let Some(id) = target_node(row, target_column_index, span)? {
                    ctx.mutator()?
                        .remove_node_label(id, label.clone())
                        .map_err(|source| graph_mutation(source, span))?;
                }
            }
            ElementKind::Edge | ElementKind::Path | ElementKind::Alias => {
                return Err(unsupported_target(element));
            }
        }
    }
    Ok(table)
}

fn execute_delete_targets(
    targets: &[DeleteTargetPlan],
    mode: DeleteMode,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let mut nodes = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut rows_since_check = 0;
    for row in table.rows() {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        for target in targets {
            match target.element {
                ElementKind::Node => {
                    if let Some(id) = target_node(row, target.target_column_index, span)?
                        && ctx.snapshot().is_node_alive(id)
                    {
                        nodes.insert(id);
                        if matches!(mode, DeleteMode::Detach) {
                            add_incident_edges(ctx, id, &mut edges);
                        }
                    }
                }
                ElementKind::Edge => {
                    if let Some(id) = target_edge(row, target.target_column_index, span)?
                        && ctx.snapshot().is_edge_alive(id)
                    {
                        edges.insert(id);
                    }
                }
                ElementKind::Path | ElementKind::Alias => {
                    return Err(unsupported_target(target.element));
                }
            }
        }
    }
    if matches!(mode, DeleteMode::Bare | DeleteMode::NoDetach) {
        ensure_delete_nodes_detached(&nodes, &edges, span, ctx)?;
    }
    ctx.mutator()?
        .delete_elements(nodes, edges)
        .map_err(|source| graph_mutation(source, span))?;
    Ok(table)
}

fn add_incident_edges(ctx: &TxContext<'_, '_>, node: NodeId, edges: &mut BTreeSet<EdgeId>) {
    if let Some(entry) = ctx.snapshot().outgoing_edges(node) {
        edges.extend(entry.iter().map(|edge| edge.edge_id));
    }
    if let Some(entry) = ctx.snapshot().incoming_edges(node) {
        edges.extend(entry.iter().map(|edge| edge.edge_id));
    }
}

fn ensure_delete_nodes_detached(
    nodes: &BTreeSet<NodeId>,
    edges: &BTreeSet<EdgeId>,
    span: SourceSpan,
    ctx: &TxContext<'_, '_>,
) -> Result<(), ExecutorError> {
    for node in nodes {
        if incident_edge_outside_delete_set(ctx, *node, edges) {
            return Err(ExecutorError::DependentObjectStillExists {
                message: "cannot delete node with incident edges (use DETACH DELETE)".to_owned(),
                span,
            });
        }
    }
    Ok(())
}

fn incident_edge_outside_delete_set(
    ctx: &TxContext<'_, '_>,
    node: NodeId,
    edges: &BTreeSet<EdgeId>,
) -> bool {
    ctx.snapshot()
        .outgoing_edges(node)
        .is_some_and(|entry| entry.iter().any(|edge| !edges.contains(&edge.edge_id)))
        || ctx
            .snapshot()
            .incoming_edges(node)
            .is_some_and(|entry| entry.iter().any(|edge| !edges.contains(&edge.edge_id)))
}

fn property_map(
    property_inits: &[PropertyInit],
    target: PropertyMapTarget,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<PropertyMap, ExecutorError> {
    let mut pairs = Vec::with_capacity(property_inits.len());
    let mut rows_since_check = 0;
    for init in property_inits {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        pairs.push((
            init.key.clone(),
            evaluator::evaluate(&init.value.expr, row, schema, ctx)?,
        ));
    }
    PropertyMap::from_pairs(pairs).map_err(|source| {
        let subclass = match target {
            PropertyMapTarget::Node => DataExceptionSubclass::NodePropertiesExceedSupportedMaximum,
            PropertyMapTarget::Edge => DataExceptionSubclass::EdgePropertiesExceedSupportedMaximum,
        };
        ExecutorError::data_exception(
            subclass,
            format!("property map construction failed: {source}"),
            property_inits
                .first()
                .map(|init| init.span)
                .unwrap_or_default(),
        )
    })
}

fn node_labels(
    label_expr: Option<&LabelExpr>,
    span: SourceSpan,
) -> Result<LabelSet, ExecutorError> {
    match label_expr {
        None => Ok(LabelSet::new()),
        Some(LabelExpr::Single(label)) => Ok(LabelSet::single(label.clone())),
        // ISO-legal label-expression forms (conjunction/disjunction) are not yet
        // implemented in the mutation surface; 42N01, not an internal-invariant break.
        Some(_) => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "INSERT label expression form",
            span,
        }),
    }
}

fn edge_label(label_expr: Option<&LabelExpr>, span: SourceSpan) -> Result<DbString, ExecutorError> {
    match label_expr {
        Some(LabelExpr::Single(label)) => Ok(label.clone()),
        // An edge insert with no label is an internal-invariant break: the binder must
        // have supplied exactly one edge label by this point.
        None => Err(ExecutorError::ImplementationDefined {
            detail: "INSERT edge label required",
        }),
        // ISO-legal edge label-expression forms are not yet implemented; 42N01.
        Some(_) => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "INSERT edge label expression form",
            span,
        }),
    }
}

fn endpoint_node(
    row: &Binding,
    endpoint: InsertEndpointRef,
    span: SourceSpan,
) -> Result<NodeId, ExecutorError> {
    match endpoint {
        InsertEndpointRef::Binding { column_index, .. } => target_node(row, column_index, span)?
            .ok_or(ExecutorError::ImplementationDefined {
                detail: "insert edge endpoint is null",
            }),
        InsertEndpointRef::InsertedNode(site_id) => {
            row.inserted_node(site_id)
                .ok_or(ExecutorError::ImplementationDefined {
                    detail: "insert edge endpoint site missing",
                })
        }
    }
}

fn edge_endpoints(
    left: NodeId,
    right: NodeId,
    direction: EdgeDirection,
    span: SourceSpan,
) -> Result<(NodeId, NodeId), ExecutorError> {
    match direction {
        EdgeDirection::Right => Ok((left, right)),
        EdgeDirection::Left => Ok((right, left)),
        // INSERT of an undirected edge is ISO-legal but not yet implemented; 42N01.
        EdgeDirection::Undirected => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "INSERT undirected edge",
            span,
        }),
    }
}

fn target_node(
    row: &Binding,
    column_index: u32,
    span: SourceSpan,
) -> Result<Option<NodeId>, ExecutorError> {
    match target_value(row, column_index, span)? {
        Value::NodeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "mutation target is not a node",
        }),
    }
}

fn target_edge(
    row: &Binding,
    column_index: u32,
    span: SourceSpan,
) -> Result<Option<EdgeId>, ExecutorError> {
    match target_value(row, column_index, span)? {
        Value::EdgeRef(id) => Ok(Some(id)),
        Value::Null => Ok(None),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "mutation target is not an edge",
        }),
    }
}

fn target_value(
    row: &Binding,
    column_index: u32,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    row.get(column_index as usize)
        .cloned()
        .ok_or_else(|| ExecutorError::InvalidReference {
            name: format!("binding column {column_index}"),
            span,
        })
}

fn extend_schema(
    schema: &mut BindingTableSchema,
    output_column_index: Option<u32>,
    output_column: Option<&BindingTableColumn>,
) -> Result<(), ExecutorError> {
    let (Some(index), Some(column)) = (output_column_index, output_column) else {
        return Ok(());
    };
    let index = index as usize;
    if index != schema.columns.len() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "mutation output column index is not append position",
        });
    }
    schema.columns.push(column.clone());
    Ok(())
}

fn set_output_value(values: &mut Vec<Value>, column_index: u32, value: Value) {
    let column_index = column_index as usize;
    values.resize(column_index + 1, Value::Null);
    values[column_index] = value;
}

fn record_insert_site(
    sites: &mut SmallVec<[(crate::InsertSiteId, NodeId); 4]>,
    site_id: crate::InsertSiteId,
    node_id: NodeId,
) {
    if let Some((_, existing)) = sites.iter_mut().find(|(site, _)| *site == site_id) {
        *existing = node_id;
    } else {
        sites.push((site_id, node_id));
    }
}

fn label_diff(
    added: impl IntoIterator<Item = DbString>,
    removed: impl IntoIterator<Item = DbString>,
    span: SourceSpan,
) -> Result<LabelDiff, ExecutorError> {
    LabelDiff::new(added, removed).map_err(|source| {
        ExecutorError::data_exception(
            DataExceptionSubclass::DataException,
            format!("label diff construction failed: {source}"),
            span,
        )
    })
}

fn property_diff(
    set: impl IntoIterator<Item = (DbString, Value)>,
    removed: impl IntoIterator<Item = DbString>,
    span: SourceSpan,
) -> Result<PropertyDiff, ExecutorError> {
    PropertyDiff::new(set, removed).map_err(|source| {
        ExecutorError::data_exception(
            DataExceptionSubclass::MultipleAssignmentsToGraphElementProperty,
            format!("property diff construction failed: {source}"),
            span,
        )
    })
}

fn graph_mutation(source: selene_graph::GraphError, span: SourceSpan) -> ExecutorError {
    ExecutorError::GraphMutation { source, span }
}

fn unsupported_target(element: ElementKind) -> ExecutorError {
    match element {
        ElementKind::Path => ExecutorError::ImplementationDefined {
            detail: "path mutation target not implemented",
        },
        ElementKind::Alias => ExecutorError::ImplementationDefined {
            detail: "alias mutation target not supported",
        },
        ElementKind::Node | ElementKind::Edge => ExecutorError::ImplementationDefined {
            detail: "mutation target element kind not supported",
        },
    }
}
