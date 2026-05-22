//! WAL schema-change replay for closed graph recovery.

use std::sync::Arc;

use selene_core::{
    IStr, LabelSet, PredefinedValueType, PropertyValueType, SchemaChange, ValueType,
};

use crate::core_provider::inconsistent;
use crate::graph_types::{
    EdgeTypeDef, GraphTypeDef, MAX_LIST_TYPE_NESTING, NodeTypeDef, PropertyDefaultValue,
    PropertyElementType, PropertyTypeDef, ValidationMode,
};

pub(super) fn replay_schema_changes(
    bound_type: &mut Option<Arc<GraphTypeDef>>,
    changes: &[SchemaChange],
) -> Result<(), crate::GraphError> {
    if changes.is_empty() {
        return Ok(());
    }
    let Some(base) = bound_type.as_deref() else {
        let variant = schema_change_variant(&changes[0]);
        return Err(crate::GraphError::Provider(inconsistent(format!(
            "WAL {variant} references missing graph type index {}",
            super::V1_BOUND_GRAPH_TYPE_INDEX
        ))));
    };
    let mut graph_type = base.clone();
    for change in changes {
        apply_schema_change(&mut graph_type, change).map_err(crate::GraphError::Provider)?;
    }
    graph_type.validate_ref()?;
    *bound_type = Some(Arc::new(graph_type));
    Ok(())
}

/// Apply a graph-type schema change to the recovered bound graph type.
///
/// This match is exhaustive by design. Variants that are handled outside
/// graph-type replay or intentionally ignored must say so inline; unsupported
/// variants reject loudly so recovery never silently drops new durable schema
/// payloads. See the `SCHEMA_CHANGE_INTENT` test table for the executable
/// contract.
fn apply_schema_change(
    graph_type: &mut GraphTypeDef,
    change: &SchemaChange,
) -> Result<(), crate::ProviderError> {
    match change {
        SchemaChange::GraphCreated { .. }
        | SchemaChange::GraphDropped { .. }
        | SchemaChange::GraphTypeCreated { .. }
        | SchemaChange::GraphTypeDropped { .. }
        | SchemaChange::RecordTypeAdded { .. } => {
            return Err(unsupported_schema_recovery(change));
        }
        SchemaChange::NodeTypeAdded { label, def, .. } => {
            // Why: snapshot sections store the bound graph type at recovery
            // index 0 in v1.0. SchemaChange.graph_type uses the durable
            // catalog id space, so replay maps every catalog type DDL event
            // onto that single recovery-state entry until multi-bound-type
            // work replaces the constant.
            let def = selene_core::NodeTypeDef::from(def.clone());
            graph_type
                .node_types
                .push(runtime_node_type_def(*label, &def)?);
        }
        SchemaChange::NodeTypeAddedV2 { label, def, .. } => {
            // See the legacy NodeTypeAdded arm above for the recovery-index
            // mapping. V2 carries live type-model fields directly.
            graph_type
                .node_types
                .push(runtime_node_type_def(*label, def)?);
        }
        SchemaChange::EdgeTypeAdded { label, def, .. } => {
            let def = selene_core::EdgeTypeDef::from(def.clone());
            graph_type
                .edge_types
                .push(runtime_edge_type_def(graph_type, *label, &def)?);
        }
        SchemaChange::EdgeTypeAddedV2 { label, def, .. } => {
            graph_type
                .edge_types
                .push(runtime_edge_type_def(graph_type, *label, def)?);
        }
        SchemaChange::NodeTypeDropped { name, .. } => {
            *graph_type = graph_type.without_node_type(*name).ok_or_else(|| {
                inconsistent(format!(
                    "WAL NodeTypeDropped references unknown type {name}"
                ))
            })?;
        }
        SchemaChange::EdgeTypeDropped { name, .. } => {
            *graph_type = graph_type.without_edge_type(*name).ok_or_else(|| {
                inconsistent(format!(
                    "WAL EdgeTypeDropped references unknown type {name}"
                ))
            })?;
        }
        SchemaChange::ProcedurePackActivated { .. }
        | SchemaChange::ProcedurePackDeprecated { .. }
        | SchemaChange::ProcedurePackDisabled { .. } => {
            // Why: legacy, never emitted; postcard discriminant pinned for ABI
            // stability.
        }
        SchemaChange::PropertyIndexCreated { .. }
        | SchemaChange::PropertyIndexDropped { .. }
        | SchemaChange::PropertyIndexCreatedNamed { .. } => {
            // Why: property-index intent is queued by apply_change and replayed
            // after primary node/edge rows materialize.
        }
        SchemaChange::ProcedurePackLifecycle { .. } => {
            // Why: lifecycle events are audit history consumed from the WAL by
            // selene-pack; CORE graph recovery has no materialized state.
        }
    }
    Ok(())
}

fn runtime_node_type_def(
    label: IStr,
    def: &selene_core::NodeTypeDef,
) -> Result<NodeTypeDef, crate::ProviderError> {
    Ok(NodeTypeDef {
        name: label,
        key_labels: def.labels.clone(),
        properties: runtime_properties(&def.properties)?,
        validation_mode: runtime_validation_mode(def.validation_mode),
    })
}

fn runtime_edge_type_def(
    graph_type: &GraphTypeDef,
    label: IStr,
    def: &selene_core::EdgeTypeDef,
) -> Result<EdgeTypeDef, crate::ProviderError> {
    let source_node_type = graph_type
        .find_node_type_index(&LabelSet::single(def.source_node_type.0))
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAdded references unknown source node type {}",
                def.source_node_type.0
            ))
        })?;
    let target_node_type = graph_type
        .find_node_type_index(&LabelSet::single(def.target_node_type.0))
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAdded references unknown target node type {}",
                def.target_node_type.0
            ))
        })?;
    Ok(EdgeTypeDef {
        name: label,
        label: def.label,
        source_node_type,
        target_node_type,
        properties: runtime_properties(&def.properties)?,
        validation_mode: runtime_validation_mode(def.validation_mode),
    })
}

fn runtime_properties(
    properties: &[selene_core::PropertyDef],
) -> Result<Vec<PropertyTypeDef>, crate::ProviderError> {
    properties
        .iter()
        .map(|property| {
            let (value_type, list_element_type) = runtime_value_type(&property.value_type)?;
            Ok(PropertyTypeDef {
                name: property.name,
                value_type,
                list_element_type,
                required: !property.nullable || property.value_type.not_null,
                default: runtime_default_value(property.default.as_ref())?,
                immutable: property.immutable,
            })
        })
        .collect()
}

fn runtime_default_value(
    value: Option<&selene_core::Value>,
) -> Result<Option<PropertyDefaultValue>, crate::ProviderError> {
    value
        .map(|value| {
            PropertyDefaultValue::from_value(value).ok_or_else(|| {
                inconsistent(format!(
                    "WAL property default uses unsupported value type {}",
                    PropertyValueType::observed_name(value)
                ))
            })
        })
        .transpose()
}

const fn runtime_validation_mode(mode: selene_core::ValidationMode) -> ValidationMode {
    match mode {
        selene_core::ValidationMode::Strict => ValidationMode::Strict,
        selene_core::ValidationMode::Warn => ValidationMode::Warn,
    }
}

fn runtime_value_type(
    value_type: &ValueType,
) -> Result<(PropertyValueType, Option<PropertyElementType>), crate::ProviderError> {
    if let Some(element_type) = value_type.list_of.as_deref() {
        return Ok((
            PropertyValueType::List,
            Some(runtime_element_type(element_type, 1)?),
        ));
    }
    if value_type.record.is_some() {
        return Ok((PropertyValueType::RecordTyped, None));
    }
    if value_type.union.is_some() {
        return Err(inconsistent(
            "WAL property definition uses unsupported union value type",
        ));
    }
    let Some(predefined) = value_type.predefined else {
        return Ok((PropertyValueType::Null, None));
    };
    Ok((runtime_predefined_value_type(predefined)?, None))
}

fn runtime_element_type(
    value_type: &ValueType,
    depth: u32,
) -> Result<PropertyElementType, crate::ProviderError> {
    if depth > MAX_LIST_TYPE_NESTING {
        return Err(inconsistent(
            "WAL property definition exceeds LIST nesting limit",
        ));
    }
    if let Some(element_type) = value_type.list_of.as_deref() {
        return Ok(PropertyElementType::List(Box::new(runtime_element_type(
            element_type,
            depth + 1,
        )?)));
    }
    if value_type.record.is_some() || value_type.union.is_some() {
        return Err(inconsistent(
            "WAL list property definition uses unsupported nested value type",
        ));
    }
    let Some(predefined) = value_type.predefined else {
        return Ok(PropertyElementType::Scalar(PropertyValueType::Null));
    };
    Ok(PropertyElementType::Scalar(runtime_predefined_value_type(
        predefined,
    )?))
}

fn runtime_predefined_value_type(
    predefined: PredefinedValueType,
) -> Result<PropertyValueType, crate::ProviderError> {
    Ok(match predefined {
        PredefinedValueType::Bool => PropertyValueType::Bool,
        PredefinedValueType::Int
        | PredefinedValueType::Int8
        | PredefinedValueType::Int16
        | PredefinedValueType::Int32
        | PredefinedValueType::Int64 => PropertyValueType::Int,
        PredefinedValueType::Int128 => PropertyValueType::Int128,
        PredefinedValueType::Uint
        | PredefinedValueType::Uint8
        | PredefinedValueType::Uint16
        | PredefinedValueType::Uint32
        | PredefinedValueType::Uint64 => PropertyValueType::Uint,
        PredefinedValueType::Uint128 => PropertyValueType::Uint128,
        PredefinedValueType::Float | PredefinedValueType::Float64 => PropertyValueType::Float,
        PredefinedValueType::Float32 => PropertyValueType::Float32,
        PredefinedValueType::Decimal => PropertyValueType::Decimal,
        PredefinedValueType::String => PropertyValueType::String,
        PredefinedValueType::Bytes => PropertyValueType::Bytes,
        PredefinedValueType::Date => PropertyValueType::Date,
        PredefinedValueType::LocalTime => PropertyValueType::LocalTime,
        PredefinedValueType::ZonedTime => PropertyValueType::ZonedTime,
        PredefinedValueType::LocalDateTime => PropertyValueType::LocalDateTime,
        PredefinedValueType::ZonedDateTime => PropertyValueType::ZonedDateTime,
        PredefinedValueType::Duration => PropertyValueType::Duration,
        PredefinedValueType::NodeRef => PropertyValueType::NodeRef,
        PredefinedValueType::EdgeRef => PropertyValueType::EdgeRef,
        PredefinedValueType::GraphRef => PropertyValueType::GraphRef,
        PredefinedValueType::TableRef => PropertyValueType::TableRef,
        PredefinedValueType::Path => PropertyValueType::Path,
        PredefinedValueType::Uuid => PropertyValueType::Uuid,
        PredefinedValueType::Extended(_) => {
            return Err(inconsistent(
                "WAL property definition uses unsupported extended value type",
            ));
        }
    })
}

/// Return the diagnostic name for a schema-change payload.
///
/// Exhaustive naming is part of the silent-skip-forbidden contract and is
/// covered by the `SCHEMA_CHANGE_INTENT` test table.
pub(super) fn schema_change_variant(change: &SchemaChange) -> &'static str {
    match change {
        SchemaChange::GraphCreated { .. } => "GraphCreated",
        SchemaChange::GraphDropped { .. } => "GraphDropped",
        SchemaChange::GraphTypeCreated { .. } => "GraphTypeCreated",
        SchemaChange::GraphTypeDropped { .. } => "GraphTypeDropped",
        SchemaChange::NodeTypeAdded { .. } => "NodeTypeAdded",
        SchemaChange::EdgeTypeAdded { .. } => "EdgeTypeAdded",
        SchemaChange::NodeTypeAddedV2 { .. } => "NodeTypeAddedV2",
        SchemaChange::EdgeTypeAddedV2 { .. } => "EdgeTypeAddedV2",
        SchemaChange::NodeTypeDropped { .. } => "NodeTypeDropped",
        SchemaChange::EdgeTypeDropped { .. } => "EdgeTypeDropped",
        SchemaChange::RecordTypeAdded { .. } => "RecordTypeAdded",
        SchemaChange::ProcedurePackActivated { .. } => "ProcedurePackActivated",
        SchemaChange::ProcedurePackDeprecated { .. } => "ProcedurePackDeprecated",
        SchemaChange::ProcedurePackDisabled { .. } => "ProcedurePackDisabled",
        SchemaChange::PropertyIndexCreated { .. } => "PropertyIndexCreated",
        SchemaChange::PropertyIndexDropped { .. } => "PropertyIndexDropped",
        SchemaChange::ProcedurePackLifecycle { .. } => "ProcedurePackLifecycle",
        SchemaChange::PropertyIndexCreatedNamed { .. } => "PropertyIndexCreatedNamed",
    }
}

pub(super) fn unsupported_schema_recovery(change: &SchemaChange) -> crate::ProviderError {
    inconsistent(format!(
        "WAL {} is not supported by CORE graph recovery; add an explicit recovery intent",
        schema_change_variant(change)
    ))
}
