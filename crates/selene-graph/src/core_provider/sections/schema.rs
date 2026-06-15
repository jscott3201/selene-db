//! Index-schema snapshot sections for the core graph provider.

use std::collections::BTreeSet;

use selene_core::{DbString, HnswIndexConfig, IvfIndexConfig};
use serde::{Deserialize, Serialize};

use crate::{
    core_provider::invalid_payload,
    graph::SeleneGraph,
    typed_index::TypedIndexKind,
    vector_index::{MAX_IVF_TARGET_CENTROIDS, VectorIndexKind},
};

use super::codec::{decode_rkyv, encode_rkyv, ensure_section_within_cap, validate_sorted_unique};

/// Entity family for a property-index schema entry.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub enum SchemaEntityKind {
    /// Node property-index registration.
    Node,
    /// Edge property-index registration.
    Edge,
}

/// Identity for an entry in the core schema section.
///
/// Schema entries currently map one-to-one with built-in node or edge property
/// index registrations. In-memory and `CORE/SCMA` wire order are lexicographic
/// by entity kind, `label.as_str()`, and `property.as_str()` for cross-process
/// stability.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct SchemaKey {
    /// Entity family the registration applies to.
    pub entity: SchemaEntityKind,
    /// Node or edge label the registration applies to.
    pub label: DbString,
    /// Property the registration applies to.
    pub property: DbString,
}

/// Persisted shape of a single schema entry.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct SchemaEntry {
    /// Indexable value kind declared at registration time.
    pub kind: TypedIndexKind,
    /// Optional explicit catalog name for the property index.
    pub name: Option<DbString>,
}

/// `CORE/SCMA` section format version byte.
///
/// Single version per the 2026-05-30 greenfield clean-break directive (no
/// shipped consumers): the on-disk layout IS the contract. A missing or
/// mismatched version byte is a hard decode error, never a silent legacy
/// fall-through - mirrors the `CORE/GTYP` collapse.
pub(in crate::core_provider) const SCMA_VERSION: u8 = 3;

/// Identity for an entry in the composite-property-index snapshot section.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct CompositeSchemaKey {
    /// Node label the composite registration applies to.
    pub label: DbString,
    /// Properties in declaration order.
    pub properties: Vec<DbString>,
}

/// Persisted shape of a composite-property-index registration.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct CompositeSchemaEntry {
    /// Indexable value kinds in declaration order.
    pub kinds: Vec<TypedIndexKind>,
    /// Optional explicit catalog name for the composite property index.
    pub name: Option<DbString>,
}

/// Identity for an entry in the vector-index snapshot section.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct VectorSchemaKey {
    /// Node label the vector registration applies to.
    pub label: DbString,
    /// Vector property the registration applies to.
    pub property: DbString,
}

/// Persisted shape of a vector-index registration.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct VectorSchemaEntry {
    /// Vector index algorithm kind.
    pub kind: VectorIndexKind,
    /// Required vector dimensionality.
    pub dimension: u32,
    /// HNSW construction config for HNSW vector indexes.
    pub hnsw_config: Option<HnswIndexConfig>,
    /// IVF construction config for IVF vector indexes.
    pub ivf_config: Option<IvfIndexConfig>,
    /// Optional explicit catalog name for the vector index.
    pub name: Option<DbString>,
}

/// Identity for an entry in the text-index snapshot section.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct TextSchemaKey {
    /// Node label the text registration applies to.
    pub label: DbString,
    /// Text property the registration applies to.
    pub property: DbString,
}

/// Persisted shape of a text-index registration.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct TextSchemaEntry {
    /// Optional explicit catalog name for the text index.
    pub name: Option<DbString>,
}

pub(in crate::core_provider) fn encode_schemas(
    graph: &SeleneGraph,
) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(SchemaKey, SchemaEntry)> = graph
        .property_index
        .iter()
        .map(|((label, property), entry)| {
            (
                SchemaKey {
                    entity: SchemaEntityKind::Node,
                    label: label.clone(),
                    property: property.clone(),
                },
                SchemaEntry {
                    kind: entry.kind(),
                    name: entry.name.clone(),
                },
            )
        })
        .collect();
    rows.extend(
        graph
            .edge_property_index
            .iter()
            .map(|((label, property), entry)| {
                (
                    SchemaKey {
                        entity: SchemaEntityKind::Edge,
                        label: label.clone(),
                        property: property.clone(),
                    },
                    SchemaEntry {
                        kind: entry.kind(),
                        name: entry.name.clone(),
                    },
                )
            }),
    );
    rows.sort_by(schema_wire_cmp);
    let mut payload = Vec::with_capacity(1);
    payload.push(SCMA_VERSION);
    payload.extend(encode_rkyv(&rows, "CORE/SCMA")?);
    ensure_section_within_cap("CORE/SCMA", payload.len())?;
    Ok(payload)
}

pub(in crate::core_provider) fn decode_schemas(
    bytes: &[u8],
) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    // Single-version clean break (greenfield, no shipped consumers): the leading
    // version byte must match `SCMA_VERSION`. A missing or mismatched byte is a
    // hard decode error - there is no legacy decoder to fall back to.
    let Some((&version, rest)) = bytes.split_first() else {
        return Err(invalid_payload(
            "CORE/SCMA section is empty (missing version byte)".to_owned(),
        ));
    };
    if version != SCMA_VERSION {
        return Err(invalid_payload(format!(
            "CORE/SCMA section version {version} is unsupported (expected {SCMA_VERSION})"
        )));
    }
    let mut rows: Vec<(SchemaKey, SchemaEntry)> = decode_rkyv(rest, "CORE/SCMA")?;
    rows.sort_unstable_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
    validate_sorted_unique(&rows, "CORE/SCMA")?;
    Ok(rows)
}

fn schema_wire_cmp<V>(lhs: &(SchemaKey, V), rhs: &(SchemaKey, V)) -> std::cmp::Ordering {
    (lhs.0.entity, lhs.0.label.as_str(), lhs.0.property.as_str()).cmp(&(
        rhs.0.entity,
        rhs.0.label.as_str(),
        rhs.0.property.as_str(),
    ))
}

pub(in crate::core_provider) fn encode_composite_schemas(
    graph: &SeleneGraph,
) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(CompositeSchemaKey, CompositeSchemaEntry)> = graph
        .composite_property_index
        .iter()
        .map(|((label, _), entry)| {
            (
                CompositeSchemaKey {
                    label: label.clone(),
                    properties: entry.declared_properties.iter().cloned().collect(),
                },
                CompositeSchemaEntry {
                    kinds: entry.kinds().iter().copied().collect(),
                    name: entry.name.clone(),
                },
            )
        })
        .collect();
    rows.sort_by(composite_schema_wire_cmp);
    encode_rkyv(&rows, "CORE/CPIX")
}

pub(in crate::core_provider) fn decode_composite_schemas(
    bytes: &[u8],
) -> Result<Vec<(CompositeSchemaKey, CompositeSchemaEntry)>, crate::ProviderError> {
    let mut rows: Vec<(CompositeSchemaKey, CompositeSchemaEntry)> =
        decode_rkyv(bytes, "CORE/CPIX")?;
    validate_composite_schema_rows(&rows)?;
    rows.sort_unstable_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
    Ok(rows)
}

fn composite_schema_wire_cmp(
    lhs: &(CompositeSchemaKey, CompositeSchemaEntry),
    rhs: &(CompositeSchemaKey, CompositeSchemaEntry),
) -> std::cmp::Ordering {
    lhs.0
        .label
        .as_str()
        .cmp(rhs.0.label.as_str())
        .then_with(|| {
            lhs.0
                .properties
                .iter()
                .map(|property| property.as_str())
                .cmp(rhs.0.properties.iter().map(|property| property.as_str()))
        })
}

pub(in crate::core_provider) fn encode_vector_schemas(
    graph: &SeleneGraph,
) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(VectorSchemaKey, VectorSchemaEntry)> = graph
        .vector_index
        .iter()
        .map(|((label, property), entry)| {
            (
                VectorSchemaKey {
                    label: label.clone(),
                    property: property.clone(),
                },
                VectorSchemaEntry {
                    kind: entry.kind(),
                    dimension: entry.dimension(),
                    hnsw_config: entry.hnsw_config(),
                    ivf_config: entry.ivf_config(),
                    name: entry.name.clone(),
                },
            )
        })
        .collect();
    rows.sort_by(vector_schema_wire_cmp);
    encode_rkyv(&rows, "CORE/VIDX")
}

pub(in crate::core_provider) fn decode_vector_schemas(
    bytes: &[u8],
) -> Result<Vec<(VectorSchemaKey, VectorSchemaEntry)>, crate::ProviderError> {
    let mut rows: Vec<(VectorSchemaKey, VectorSchemaEntry)> = decode_rkyv(bytes, "CORE/VIDX")?;
    rows.sort_unstable_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
    validate_vector_schema_rows(&rows)?;
    Ok(rows)
}

pub(in crate::core_provider) fn encode_text_schemas(
    graph: &SeleneGraph,
) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(TextSchemaKey, TextSchemaEntry)> = graph
        .text_index
        .iter()
        .map(|((label, property), entry)| {
            (
                TextSchemaKey {
                    label: label.clone(),
                    property: property.clone(),
                },
                TextSchemaEntry {
                    name: entry.name.clone(),
                },
            )
        })
        .collect();
    rows.sort_by(text_schema_wire_cmp);
    encode_rkyv(&rows, "CORE/TIDX")
}

pub(in crate::core_provider) fn decode_text_schemas(
    bytes: &[u8],
) -> Result<Vec<(TextSchemaKey, TextSchemaEntry)>, crate::ProviderError> {
    let mut rows: Vec<(TextSchemaKey, TextSchemaEntry)> = decode_rkyv(bytes, "CORE/TIDX")?;
    rows.sort_unstable_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
    validate_sorted_unique(&rows, "CORE/TIDX")?;
    Ok(rows)
}

fn vector_schema_wire_cmp(
    lhs: &(VectorSchemaKey, VectorSchemaEntry),
    rhs: &(VectorSchemaKey, VectorSchemaEntry),
) -> std::cmp::Ordering {
    (lhs.0.label.as_str(), lhs.0.property.as_str())
        .cmp(&(rhs.0.label.as_str(), rhs.0.property.as_str()))
}

fn text_schema_wire_cmp(
    lhs: &(TextSchemaKey, TextSchemaEntry),
    rhs: &(TextSchemaKey, TextSchemaEntry),
) -> std::cmp::Ordering {
    (lhs.0.label.as_str(), lhs.0.property.as_str())
        .cmp(&(rhs.0.label.as_str(), rhs.0.property.as_str()))
}

fn validate_composite_schema_rows(
    rows: &[(CompositeSchemaKey, CompositeSchemaEntry)],
) -> Result<(), crate::ProviderError> {
    let mut seen = BTreeSet::new();
    for (key, entry) in rows {
        if key.properties.len() < 2 {
            return Err(invalid_payload(format!(
                "CORE/CPIX row for label {} has fewer than two properties",
                key.label
            )));
        }
        if key.properties.len() != entry.kinds.len() {
            return Err(invalid_payload(format!(
                "CORE/CPIX row for label {} has {} properties but {} kinds",
                key.label,
                key.properties.len(),
                entry.kinds.len()
            )));
        }
        let mut canonical = key.properties.clone();
        canonical.sort_unstable();
        if canonical.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid_payload(format!(
                "CORE/CPIX row for label {} repeats a property",
                key.label
            )));
        }
        if !seen.insert((key.label.clone(), canonical)) {
            return Err(invalid_payload(format!(
                "CORE/CPIX rows contain duplicate composite registration for label {}",
                key.label
            )));
        }
    }
    Ok(())
}

fn validate_vector_schema_rows(
    rows: &[(VectorSchemaKey, VectorSchemaEntry)],
) -> Result<(), crate::ProviderError> {
    validate_sorted_unique(rows, "CORE/VIDX")?;
    for (key, entry) in rows {
        if entry.dimension == 0 {
            return Err(invalid_payload(format!(
                "CORE/VIDX row for ({}, {}) has zero vector dimension",
                key.label, key.property
            )));
        }
        if entry.kind.hnsw_metric().is_some() != entry.hnsw_config.is_some() {
            return Err(invalid_payload(format!(
                "CORE/VIDX row for ({}, {}) has inconsistent HNSW config",
                key.label, key.property
            )));
        }
        if entry.kind.ivf_metric().is_some() {
            if let Some(config) = entry.ivf_config
                && (config.target_centroids == 0
                    || config.target_centroids > MAX_IVF_TARGET_CENTROIDS)
            {
                return Err(invalid_payload(format!(
                    "CORE/VIDX row for ({}, {}) has invalid IVF config",
                    key.label, key.property
                )));
            }
        } else if entry.ivf_config.is_some() {
            return Err(invalid_payload(format!(
                "CORE/VIDX row for ({}, {}) has inconsistent IVF config",
                key.label, key.property
            )));
        }
        if let Some(config) = entry.hnsw_config
            && (config.max_neighbors == 0
                || config.ef_construction == 0
                || config.ef_construction < config.max_neighbors)
        {
            return Err(invalid_payload(format!(
                "CORE/VIDX row for ({}, {}) has invalid HNSW config",
                key.label, key.property
            )));
        }
    }
    Ok(())
}
