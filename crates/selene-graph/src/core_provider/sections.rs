//! rkyv section payloads for the core graph provider.

use std::sync::Arc;

use rkyv::{
    Place,
    rancor::Fallible,
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use selene_core::{EdgeId, IStr, LabelSet, NodeId, PropertyMap};
use selene_persist::MAX_SECTION_PAYLOAD_BYTES;
use serde::{Deserialize, Serialize};

use crate::core_provider::{inconsistent, invalid_payload, serialization_failed};
use crate::graph::{GraphMeta, SeleneGraph};
use crate::typed_index::TypedIndexKind;

struct ArcBytes;

impl ArchiveWith<Arc<[u8]>> for ArcBytes {
    type Archived = ArchivedVec<u8>;
    type Resolver = VecResolver;

    fn resolve_with(field: &Arc<[u8]>, resolver: Self::Resolver, out: Place<Self::Archived>) {
        ArchivedVec::resolve_from_slice(field.as_ref(), resolver, out);
    }
}

impl<S> SerializeWith<Arc<[u8]>, S> for ArcBytes
where
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(field: &Arc<[u8]>, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        ArchivedVec::serialize_from_slice(field.as_ref(), serializer)
    }
}

impl<D> DeserializeWith<ArchivedVec<u8>, Arc<[u8]>, D> for ArcBytes
where
    D: Fallible + ?Sized,
{
    fn deserialize_with(
        field: &ArchivedVec<u8>,
        _deserializer: &mut D,
    ) -> Result<Arc<[u8]>, D::Error> {
        Ok(Arc::from(field.as_slice()))
    }
}

/// Graph metadata section payload.
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

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct NodeArchiveRow {
    labels: LabelSet,
    #[rkyv(with = ArcBytes)]
    properties_blob: Arc<[u8]>,
    alive: bool,
}

impl NodeArchiveRow {
    fn from_runtime(row: NodeRow, section: &'static str) -> Result<Self, crate::ProviderError> {
        Ok(Self {
            labels: row.labels,
            properties_blob: encode_properties_blob(&row.properties, section)?,
            alive: row.alive,
        })
    }

    fn into_runtime(self, section: &'static str) -> Result<NodeRow, crate::ProviderError> {
        Ok(NodeRow {
            labels: self.labels,
            properties: decode_properties_blob(&self.properties_blob, section)?,
            alive: self.alive,
        })
    }
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct EdgeArchiveRow {
    label: IStr,
    source: NodeId,
    target: NodeId,
    #[rkyv(with = ArcBytes)]
    properties_blob: Arc<[u8]>,
    alive: bool,
}

impl EdgeArchiveRow {
    fn from_runtime(row: EdgeRow, section: &'static str) -> Result<Self, crate::ProviderError> {
        Ok(Self {
            label: row.label,
            source: row.source,
            target: row.target,
            properties_blob: encode_properties_blob(&row.properties, section)?,
            alive: row.alive,
        })
    }

    fn into_runtime(self, section: &'static str) -> Result<EdgeRow, crate::ProviderError> {
        Ok(EdgeRow {
            label: self.label,
            source: self.source,
            target: self.target,
            properties: decode_properties_blob(&self.properties_blob, section)?,
            alive: self.alive,
        })
    }
}

/// Identity for an entry in the core schema section.
///
/// In v1.0, schema entries map one-to-one with built-in property index
/// registrations. BRIEF-15 (closed-graph types) extends the [`SchemaEntry`]
/// payload but keeps the section sub-tag stable.
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
pub struct SchemaKey {
    /// Node label the registration applies to.
    pub label: IStr,
    /// Property the registration applies to.
    pub property: IStr,
}

/// Persisted shape of a single schema entry.
#[derive(
    Clone,
    Copy,
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
}

pub(super) fn encode_meta(
    meta: &GraphMeta,
    sequence: u64,
) -> Result<Vec<u8>, crate::ProviderError> {
    encode_rkyv(
        &MetaPayload {
            meta: meta.clone(),
            sequence,
        },
        "CORE/META",
    )
}

pub(super) fn decode_meta(bytes: &[u8]) -> Result<MetaPayload, crate::ProviderError> {
    decode_rkyv(bytes, "CORE/META")
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
        let runtime = NodeRow {
            labels: labels.clone(),
            properties: properties.clone(),
            alive: graph.node_store.is_alive(row),
        };
        rows.push((
            NodeId::new(row as u64 + 1),
            NodeArchiveRow::from_runtime(runtime, "CORE/NODE")?,
        ));
    }
    encode_rkyv(&rows, "CORE/NODE")
}

pub(super) fn decode_nodes(bytes: &[u8]) -> Result<Vec<(NodeId, NodeRow)>, crate::ProviderError> {
    let rows: Vec<(NodeId, NodeArchiveRow)> = decode_rkyv(bytes, "CORE/NODE")?;
    validate_sorted_unique(&rows, "CORE/NODE")?;
    rows.into_iter()
        .map(|(id, row)| row.into_runtime("CORE/NODE").map(|row| (id, row)))
        .collect()
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
        let runtime = EdgeRow {
            label: *label,
            source: *source,
            target: *target,
            properties: properties.clone(),
            alive: graph.edge_store.is_alive(row),
        };
        rows.push((
            EdgeId::new(row as u64 + 1),
            EdgeArchiveRow::from_runtime(runtime, "CORE/EDGE")?,
        ));
    }
    encode_rkyv(&rows, "CORE/EDGE")
}

pub(super) fn decode_edges(bytes: &[u8]) -> Result<Vec<(EdgeId, EdgeRow)>, crate::ProviderError> {
    let rows: Vec<(EdgeId, EdgeArchiveRow)> = decode_rkyv(bytes, "CORE/EDGE")?;
    validate_sorted_unique(&rows, "CORE/EDGE")?;
    rows.into_iter()
        .map(|(id, row)| row.into_runtime("CORE/EDGE").map(|row| (id, row)))
        .collect()
}

pub(super) fn encode_schemas(graph: &SeleneGraph) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(SchemaKey, SchemaEntry)> = graph
        .property_index
        .iter()
        .map(|((label, property), index)| {
            (
                SchemaKey {
                    label: *label,
                    property: *property,
                },
                SchemaEntry { kind: index.kind() },
            )
        })
        .collect();
    rows.sort_by_key(|(key, _)| *key);
    encode_rkyv(&rows, "CORE/SCMA")
}

pub(super) fn decode_schemas(
    bytes: &[u8],
) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    let rows: Vec<(SchemaKey, SchemaEntry)> = decode_rkyv(bytes, "CORE/SCMA")?;
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

fn encode_rkyv<T>(value: &T, section: &'static str) -> Result<Vec<u8>, crate::ProviderError>
where
    T: for<'a> rkyv::Serialize<
            rkyv::api::high::HighSerializer<
                rkyv::util::AlignedVec,
                rkyv::ser::allocator::ArenaHandle<'a>,
                rkyv::rancor::Error,
            >,
        >,
{
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(value)
        .map_err(|error| serialization_failed(format!("{section} rkyv encode failed: {error}")))?
        .into_vec();
    ensure_section_within_cap(section, bytes.len())?;
    Ok(bytes)
}

fn decode_rkyv<T>(bytes: &[u8], section: &'static str) -> Result<T, crate::ProviderError>
where
    T: rkyv::Archive,
    T::Archived: for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
        + rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
{
    ensure_section_within_cap(section, bytes.len())?;
    rkyv::from_bytes::<T, rkyv::rancor::Error>(bytes).map_err(|error| {
        invalid_payload(format!("{section} rkyv bytecheck/decode failed: {error}"))
    })
}

fn encode_properties_blob(
    properties: &PropertyMap,
    section: &'static str,
) -> Result<Arc<[u8]>, crate::ProviderError> {
    let bytes = postcard::to_stdvec(properties).map_err(|error| {
        serialization_failed(format!(
            "{section} property postcard encode failed: {error}"
        ))
    })?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

fn decode_properties_blob(
    bytes: &[u8],
    section: &'static str,
) -> Result<PropertyMap, crate::ProviderError> {
    postcard::from_bytes(bytes).map_err(|error| {
        invalid_payload(format!(
            "{section} property postcard decode failed: {error}"
        ))
    })
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
