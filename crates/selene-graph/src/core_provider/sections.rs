//! Postcard section payloads for the core graph provider.

use selene_core::{EdgeId, GraphId, IStr, LabelSet, NodeId, PropertyMap};
use selene_persist::MAX_SECTION_PAYLOAD_BYTES;
use serde::{Deserialize, Serialize};

use crate::core_provider::{inconsistent, invalid_payload, serialization_failed};
use crate::graph::{GraphMeta, SeleneGraph};

/// Graph metadata section payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetaPayload {
    /// Graph snapshot metadata.
    pub meta: GraphMeta,
    /// Persistence sequence associated with the metadata payload.
    pub sequence: u64,
}

/// Serialized node-store row.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NodeRow {
    /// Node labels stored in the row.
    pub labels: LabelSet,
    /// Node properties stored in the row.
    pub properties: PropertyMap,
    /// Whether the row is live.
    pub alive: bool,
}

/// Serialized edge-store row.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EdgeRow {
    /// Edge label.
    pub label: IStr,
    /// Source node ID.
    pub source: NodeId,
    /// Target node ID.
    pub target: NodeId,
    /// Edge properties.
    pub properties: PropertyMap,
    /// Whether the row is live.
    pub alive: bool,
}

/// Placeholder key for the core schema section.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SchemaKey {
    /// Opaque schema key ID. BRIEF-15 gives this real meaning.
    pub id: u64,
}

/// Placeholder value for the core schema section.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchemaEntry {
    /// Opaque schema bytes. Empty in v1.0.
    pub payload: Vec<u8>,
}

pub(super) fn encode_meta(
    meta: &GraphMeta,
    sequence: u64,
) -> Result<Vec<u8>, crate::ProviderError> {
    encode_postcard(
        &MetaPayload {
            meta: meta.clone(),
            sequence,
        },
        "CORE/META",
    )
}

pub(super) fn decode_meta(bytes: &[u8]) -> Result<MetaPayload, crate::ProviderError> {
    decode_postcard(bytes, "CORE/META")
}

pub(super) fn encode_nodes(graph: &SeleneGraph) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows = Vec::with_capacity(graph.node_store.len());
    for row_index in 0..graph.node_store.len() {
        let row = u32::try_from(row_index).map_err(|_| {
            inconsistent(format!(
                "node row index {row_index} exceeds u32::MAX; core snapshot sections use v1 row indexes"
            ))
        })?;
        let labels =
            graph.node_store.labels.get(row_index).ok_or_else(|| {
                inconsistent(format!("node labels column missing row {row_index}"))
            })?;
        let properties = graph.node_store.properties.get(row_index).ok_or_else(|| {
            inconsistent(format!("node properties column missing row {row_index}"))
        })?;
        rows.push((
            NodeId::new(row as u64 + 1),
            NodeRow {
                labels: labels.clone(),
                properties: properties.clone(),
                alive: graph.node_store.is_alive(row),
            },
        ));
    }
    encode_postcard(&rows, "CORE/NODE")
}

pub(super) fn decode_nodes(bytes: &[u8]) -> Result<Vec<(NodeId, NodeRow)>, crate::ProviderError> {
    let rows: Vec<(NodeId, NodeRow)> = decode_postcard(bytes, "CORE/NODE")?;
    validate_sorted_unique(&rows, "CORE/NODE")?;
    Ok(rows)
}

pub(super) fn encode_edges(graph: &SeleneGraph) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows = Vec::with_capacity(graph.edge_store.len());
    for row_index in 0..graph.edge_store.len() {
        let row = u32::try_from(row_index).map_err(|_| {
            inconsistent(format!(
                "edge row index {row_index} exceeds u32::MAX; core snapshot sections use v1 row indexes"
            ))
        })?;
        let label =
            graph.edge_store.label.get(row_index).ok_or_else(|| {
                inconsistent(format!("edge label column missing row {row_index}"))
            })?;
        let source =
            graph.edge_store.source.get(row_index).ok_or_else(|| {
                inconsistent(format!("edge source column missing row {row_index}"))
            })?;
        let target =
            graph.edge_store.target.get(row_index).ok_or_else(|| {
                inconsistent(format!("edge target column missing row {row_index}"))
            })?;
        let properties = graph.edge_store.properties.get(row_index).ok_or_else(|| {
            inconsistent(format!("edge properties column missing row {row_index}"))
        })?;
        rows.push((
            EdgeId::new(row as u64 + 1),
            EdgeRow {
                label: *label,
                source: *source,
                target: *target,
                properties: properties.clone(),
                alive: graph.edge_store.is_alive(row),
            },
        ));
    }
    encode_postcard(&rows, "CORE/EDGE")
}

pub(super) fn decode_edges(bytes: &[u8]) -> Result<Vec<(EdgeId, EdgeRow)>, crate::ProviderError> {
    let rows: Vec<(EdgeId, EdgeRow)> = decode_postcard(bytes, "CORE/EDGE")?;
    validate_sorted_unique(&rows, "CORE/EDGE")?;
    Ok(rows)
}

pub(super) fn encode_schemas() -> Result<Vec<u8>, crate::ProviderError> {
    encode_postcard::<Vec<(SchemaKey, SchemaEntry)>>(&Vec::new(), "CORE/SCMA")
}

pub(super) fn decode_schemas(
    bytes: &[u8],
) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    let rows: Vec<(SchemaKey, SchemaEntry)> = decode_postcard(bytes, "CORE/SCMA")?;
    validate_sorted_unique(&rows, "CORE/SCMA")?;
    Ok(rows)
}

pub(super) fn ensure_section_within_cap(
    section: &'static str,
    len: usize,
) -> Result<(), crate::ProviderError> {
    if len > MAX_SECTION_PAYLOAD_BYTES {
        return Err(inconsistent(format!(
            "{section} core section exceeds 1 GiB cap; multi-section split is a future v1.x hardening"
        )));
    }
    Ok(())
}

fn encode_postcard<T: Serialize>(
    value: &T,
    section: &'static str,
) -> Result<Vec<u8>, crate::ProviderError> {
    let bytes = postcard::to_stdvec(value).map_err(|error| {
        serialization_failed(format!("{section} postcard encode failed: {error}"))
    })?;
    ensure_section_within_cap(section, bytes.len())?;
    Ok(bytes)
}

fn decode_postcard<'a, T: Deserialize<'a>>(
    bytes: &'a [u8],
    section: &'static str,
) -> Result<T, crate::ProviderError> {
    ensure_section_within_cap(section, bytes.len())?;
    postcard::from_bytes(bytes)
        .map_err(|error| invalid_payload(format!("{section} postcard decode failed: {error}")))
}

fn validate_sorted_unique<K, V>(
    rows: &[(K, V)],
    section: &'static str,
) -> Result<(), crate::ProviderError>
where
    K: Ord + std::fmt::Debug,
{
    for pair in rows.windows(2) {
        if pair[0].0 >= pair[1].0 {
            return Err(invalid_payload(format!(
                "{section} rows must be strictly sorted by key with no duplicates; observed {:?} then {:?}",
                pair[0].0, pair[1].0
            )));
        }
    }
    Ok(())
}

pub(super) fn default_recovered_meta() -> GraphMeta {
    GraphMeta {
        graph_id: GraphId::new(1),
        generation: 0,
        next_node_id: 1,
        next_edge_id: 1,
    }
}
