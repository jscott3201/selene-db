//! WAL schema-change replay for closed graph recovery.

use std::sync::Arc;

use selene_core::{
    DbString, EdgeEndpointDef as CoreEdgeEndpointDef, LabelSet, PredefinedValueType,
    PropertyValueType, SchemaChange, ValueType,
};

use crate::core_provider::inconsistent;
use crate::graph_types::{
    EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, MAX_LIST_TYPE_NESTING, MAX_RECORD_TYPE_NESTING,
    NodeTypeDef, PropertyDefaultValue, PropertyElementType, PropertyTypeDef, RecordFieldType,
    RecordFieldTypeDef, RecordFieldTypes, ValidationMode,
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
                .push(runtime_node_type_def(label.clone(), &def)?);
        }
        SchemaChange::NodeTypeAddedV2 { label, def, .. } => {
            // See the legacy NodeTypeAdded arm above for the recovery-index
            // mapping. V2 carries live type-model fields directly.
            graph_type
                .node_types
                .push(runtime_node_type_def(label.clone(), def)?);
        }
        SchemaChange::EdgeTypeAdded { label, def, .. } => {
            let def = selene_core::EdgeTypeDef::from(def.clone());
            graph_type
                .edge_types
                .push(runtime_edge_type_def(graph_type, label.clone(), &def)?);
        }
        SchemaChange::EdgeTypeAddedV2 { label, def, .. } => {
            graph_type
                .edge_types
                .push(runtime_edge_type_def(graph_type, label.clone(), def)?);
        }
        SchemaChange::NodeTypeDropped { name, .. } => {
            *graph_type = graph_type.without_node_type(name.clone()).ok_or_else(|| {
                inconsistent(format!(
                    "WAL NodeTypeDropped references unknown type {name}"
                ))
            })?;
        }
        SchemaChange::EdgeTypeDropped { name, .. } => {
            *graph_type = graph_type.without_edge_type(name.clone()).ok_or_else(|| {
                inconsistent(format!(
                    "WAL EdgeTypeDropped references unknown type {name}"
                ))
            })?;
        }
        SchemaChange::PropertyIndexCreated { .. }
        | SchemaChange::PropertyIndexDropped { .. }
        | SchemaChange::PropertyIndexCreatedNamed { .. }
        | SchemaChange::CompositePropertyIndexCreated { .. }
        | SchemaChange::CompositePropertyIndexDropped { .. }
        | SchemaChange::VectorIndexCreated { .. }
        | SchemaChange::VectorIndexDropped { .. }
        | SchemaChange::TextIndexCreated { .. }
        | SchemaChange::TextIndexDropped { .. } => {
            // Why: index intent is queued by apply_change and replayed
            // after primary node/edge rows materialize.
        }
    }
    Ok(())
}

fn runtime_node_type_def(
    label: DbString,
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
    label: DbString,
    def: &selene_core::EdgeTypeDef,
) -> Result<EdgeTypeDef, crate::ProviderError> {
    Ok(EdgeTypeDef {
        name: label,
        label: def.label.clone(),
        source_node_type: runtime_edge_endpoint_def(graph_type, &def.source_node_type, "source")?,
        target_node_type: runtime_edge_endpoint_def(graph_type, &def.target_node_type, "target")?,
        properties: runtime_properties(&def.properties)?,
        validation_mode: runtime_validation_mode(def.validation_mode),
    })
}

fn runtime_edge_endpoint_def(
    graph_type: &GraphTypeDef,
    endpoint: &CoreEdgeEndpointDef,
    role: &str,
) -> Result<EdgeEndpointDef, crate::ProviderError> {
    match endpoint {
        CoreEdgeEndpointDef::Any => Ok(EdgeEndpointDef::Any),
        CoreEdgeEndpointDef::NodeType(node_type) => Ok(EdgeEndpointDef::NodeType(
            resolve_node_type_ref(graph_type, node_type.clone(), role)?,
        )),
        CoreEdgeEndpointDef::OneOf(node_types) => {
            // Resolve each WAL NodeTypeRef to a storage index via the same
            // resilient lookup the single-NodeType arm uses. Sort + dedupe +
            // singleton-collapse is applied AFTER resolution by passing the
            // gathered indices through `EdgeEndpointDef::one_of`. This handles
            // the F5 case where the recovered GraphTypeDef has node types in a
            // different order from the original snapshot.
            let mut indices: Vec<u32> = Vec::with_capacity(node_types.len());
            for node_type in node_types {
                indices.push(resolve_node_type_ref(graph_type, node_type.clone(), role)?);
            }
            Ok(EdgeEndpointDef::one_of(indices))
        }
    }
}

fn resolve_node_type_ref(
    graph_type: &GraphTypeDef,
    node_type: selene_core::NodeTypeRef,
    role: &str,
) -> Result<u32, crate::ProviderError> {
    graph_type
        .find_node_type_index(&LabelSet::single(node_type.0.clone()))
        .or_else(|| graph_type.node_type_index_for(node_type.0.clone()))
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAdded references unknown {role} node type {}",
                node_type.0
            ))
        })
}

fn runtime_properties(
    properties: &[selene_core::PropertyDef],
) -> Result<Vec<PropertyTypeDef>, crate::ProviderError> {
    properties
        .iter()
        .map(|property| {
            // Why: a RECORD property's structure rides `PropertyDef.record_fields`
            // (structural-inline), NOT `ValueType.record`. `record_fields` jointly encodes
            // three states so recovery faithfully restores the committed catalog (D11-class
            // live-vs-recovered consistency): `None` ⇒ not a record; `Some(Open)` ⇒ an
            // open/bare `RECORD` (RecordTyped with no field constraints — without this
            // marker its all-`None` `ValueType` would degrade to scalar `Null` on replay);
            // `Some(Closed)` ⇒ a closed/typed `RECORD{..}`. The coarse `RecordTyped` tag is
            // derived here (not stored on `ValueType`) so commit-time GG02 validation treats
            // it correctly. Per ISO 39075:2024 §18.9/§18.10 (GV46/GV47/GV48).
            let (value_type, list_element_type, record_field_types) =
                match property.record_fields.as_deref() {
                    Some(selene_core::RecordFieldStructure::Open) => {
                        (PropertyValueType::RecordTyped, None, None)
                    }
                    Some(selene_core::RecordFieldStructure::Closed(defs)) => (
                        PropertyValueType::RecordTyped,
                        None,
                        Some(runtime_record_field_types(defs, 1)?),
                    ),
                    None => {
                        let (value_type, list_element_type) =
                            runtime_value_type(&property.value_type)?;
                        (value_type, list_element_type, None)
                    }
                };
            Ok(PropertyTypeDef {
                name: property.name.clone(),
                value_type,
                list_element_type,
                required: !property.nullable || property.value_type.not_null,
                default: runtime_default_value(property.default.as_ref())?,
                immutable: property.immutable,
                unique: property.unique,
                record_field_types,
            })
        })
        .collect()
}

fn runtime_record_field_types(
    defs: &[selene_core::RecordFieldStructureDef],
    depth: u32,
) -> Result<RecordFieldTypes, crate::ProviderError> {
    if depth > MAX_RECORD_TYPE_NESTING {
        return Err(inconsistent(
            "WAL property definition exceeds RECORD nesting limit",
        ));
    }
    let fields = defs
        .iter()
        .map(|field| {
            Ok(RecordFieldTypeDef {
                name: field.name.clone(),
                field_type: runtime_record_field_type(&field.field_type, depth)?,
                required: field.required,
            })
        })
        .collect::<Result<Vec<_>, crate::ProviderError>>()?;
    Ok(RecordFieldTypes(fields))
}

fn runtime_record_field_type(
    field_type: &selene_core::RecordFieldStructureType,
    depth: u32,
) -> Result<RecordFieldType, crate::ProviderError> {
    match field_type {
        selene_core::RecordFieldStructureType::Scalar(value_type) => {
            Ok(RecordFieldType::Scalar(*value_type))
        }
        selene_core::RecordFieldStructureType::List(inner) => Ok(RecordFieldType::List(Box::new(
            runtime_record_field_type(inner, depth + 1)?,
        ))),
        selene_core::RecordFieldStructureType::Record(inner) => match inner.as_ref() {
            selene_core::RecordFieldStructure::Closed(defs) => Ok(RecordFieldType::Record(
                Box::new(runtime_record_field_types(defs, depth + 1)?),
            )),
            selene_core::RecordFieldStructure::Open => Ok(RecordFieldType::OpenRecord),
        },
    }
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
        PredefinedValueType::DurationYearToMonth => PropertyValueType::DurationYearToMonth,
        PredefinedValueType::DurationDayToSecond => PropertyValueType::DurationDayToSecond,
        PredefinedValueType::NodeRef => PropertyValueType::NodeRef,
        PredefinedValueType::EdgeRef => PropertyValueType::EdgeRef,
        PredefinedValueType::GraphRef => PropertyValueType::GraphRef,
        PredefinedValueType::TableRef => PropertyValueType::TableRef,
        PredefinedValueType::Path => PropertyValueType::Path,
        PredefinedValueType::Uuid => PropertyValueType::Uuid,
        PredefinedValueType::Vector => PropertyValueType::Vector,
        PredefinedValueType::Json => PropertyValueType::Json,
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
        SchemaChange::PropertyIndexCreated { .. } => "PropertyIndexCreated",
        SchemaChange::PropertyIndexDropped { .. } => "PropertyIndexDropped",
        SchemaChange::PropertyIndexCreatedNamed { .. } => "PropertyIndexCreatedNamed",
        SchemaChange::CompositePropertyIndexCreated { .. } => "CompositePropertyIndexCreated",
        SchemaChange::CompositePropertyIndexDropped { .. } => "CompositePropertyIndexDropped",
        SchemaChange::VectorIndexCreated { .. } => "VectorIndexCreated",
        SchemaChange::VectorIndexDropped { .. } => "VectorIndexDropped",
        SchemaChange::TextIndexCreated { .. } => "TextIndexCreated",
        SchemaChange::TextIndexDropped { .. } => "TextIndexDropped",
    }
}

pub(super) fn unsupported_schema_recovery(change: &SchemaChange) -> crate::ProviderError {
    inconsistent(format!(
        "WAL {} is not supported by CORE graph recovery; add an explicit recovery intent",
        schema_change_variant(change)
    ))
}
