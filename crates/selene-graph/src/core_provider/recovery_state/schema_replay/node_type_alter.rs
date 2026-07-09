//! Additive node-type ALTER validation during WAL replay.

use selene_core::{DbString, PropertyDef};

use crate::{GraphTypeDef, ProviderError, core_provider::inconsistent};

pub(super) fn apply(
    graph_type: &mut GraphTypeDef,
    label: &DbString,
    properties: &[PropertyDef],
) -> Result<(), ProviderError> {
    let index = graph_type
        .node_type_index_for(label.clone())
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL NodeTypeAlteredV2 references unknown node type {label}"
            ))
        })? as usize;
    let mut merged = graph_type.node_types[index].properties.clone();
    for property in properties {
        let decoded = super::runtime_property(property)?;
        if let Some(existing) = merged
            .iter()
            .find(|candidate| candidate.name == decoded.name)
        {
            if existing == &decoded {
                continue;
            }
            return Err(non_additive(
                label,
                &format!("redefines property {}", decoded.name),
            ));
        }
        if decoded.required {
            return Err(non_additive(
                label,
                &format!("appends required property {}", decoded.name),
            ));
        }
        crate::type_validator::validate_property_default(&decoded).map_err(|error| {
            non_additive(
                label,
                &format!("property {} has invalid default: {error}", decoded.name),
            )
        })?;
        merged.push(decoded);
    }

    graph_type.node_types[index].properties = merged;
    Ok(())
}

fn non_additive(label: &DbString, reason: &str) -> ProviderError {
    inconsistent(format!(
        "WAL NodeTypeAlteredV2 for {label} is not additive: {reason}"
    ))
}
