//! rkyv section payloads for the core graph provider.
//!
//! Section payload format: rkyv 0.8 archives over sorted `Vec<(K, Row)>`
//! intermediates per spec 04 §4.3 / decision D14. Per-row property bags
//! are postcard-encoded inside an `Arc<[u8]>` field on each archived row until
//! every stored `Value` variant has rkyv archivability.
//!
//! Compatibility note: older in-tree snapshots wrote these same
//! `CORE/META|NODE|EDGE|SCMA` sub-tags as postcard payloads. The snapshot
//! envelope version stays `1` because selene-db has never shipped — see
//! Spec 04 §4.6 ("never shipped") — and there are no real on-disk graph
//! instances pre-dating this change. When v1.0 ships, any further format
//! change to a CORE section bumps the envelope version so old snapshots fail
//! with `PersistError::UnsupportedVersion` instead of garbled bytes.
//!
//! `CORE/SCMA` schema rows are stored in memory by [`IStr`] handle order via
//! [`SchemaKey::Ord`]. Their wire order is canonical lexicographic order by
//! `(label.as_str(), property.as_str())`; decode re-sorts to the receiver's
//! local handle order before duplicate validation.

use std::collections::BTreeSet;
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

mod gtyp;

pub(super) use gtyp::{decode_graph_types, encode_graph_types};

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
    /// Graph identifier.
    pub graph_id: selene_core::GraphId,
    /// Published generation counter.
    pub generation: u64,
    /// Next node ID to allocate.
    pub next_node_id: u64,
    /// Next edge ID to allocate.
    pub next_edge_id: u64,
    /// Index into the `CORE/GTYP` table for closed graphs.
    pub bound_type_index: Option<u32>,
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
/// registrations. In-memory order follows local [`IStr`] handles; the
/// `CORE/SCMA` wire order is lexicographic by `label.as_str()` and
/// `property.as_str()` for cross-process stability.
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
    /// Optional explicit catalog name for the property index.
    pub name: Option<IStr>,
}

/// Legacy v1 `CORE/SCMA` row with no stored index name.
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
pub(super) struct SchemaEntryV1 {
    /// Indexable value kind declared at registration time.
    pub kind: TypedIndexKind,
}

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
struct SchemaEntryV2 {
    kind: TypedIndexKind,
    name: Option<IStr>,
}

const SCMA_V2_MAGIC: u8 = 0xA5;

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
    pub label: IStr,
    /// Properties in declaration order.
    pub properties: Vec<IStr>,
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
    pub name: Option<IStr>,
}

pub(super) fn encode_meta(
    meta: &GraphMeta,
    sequence: u64,
) -> Result<Vec<u8>, crate::ProviderError> {
    encode_rkyv(
        &MetaPayload {
            graph_id: meta.graph_id,
            generation: meta.generation,
            next_node_id: meta.next_node_id,
            next_edge_id: meta.next_edge_id,
            bound_type_index: meta.bound_type.as_ref().map(|_| 0),
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
    let mut rows: Vec<(SchemaKey, SchemaEntryV2)> = graph
        .property_index
        .iter()
        .map(|((label, property), entry)| {
            (
                SchemaKey {
                    label: *label,
                    property: *property,
                },
                SchemaEntryV2 {
                    kind: entry.kind(),
                    name: entry.name,
                },
            )
        })
        .collect();
    rows.sort_by(schema_wire_cmp);
    let mut payload = Vec::with_capacity(1);
    payload.push(SCMA_V2_MAGIC);
    payload.extend(encode_rkyv(&rows, "CORE/SCMA")?);
    ensure_section_within_cap("CORE/SCMA", payload.len())?;
    Ok(payload)
}

pub(super) fn decode_schemas(
    bytes: &[u8],
) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    let mut rows = if bytes.first() == Some(&SCMA_V2_MAGIC) {
        decode_schema_v2(&bytes[1..]).or_else(|_| decode_schema_v1(bytes))?
    } else {
        decode_schema_v1(bytes)?
    };
    rows.sort_unstable_by_key(|(key, _)| *key);
    validate_sorted_unique(&rows, "CORE/SCMA")?;
    Ok(rows)
}

fn decode_schema_v1(bytes: &[u8]) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    let rows: Vec<(SchemaKey, SchemaEntryV1)> = decode_rkyv(bytes, "CORE/SCMA")?;
    Ok(rows
        .into_iter()
        .map(|(key, entry)| {
            (
                key,
                SchemaEntry {
                    kind: entry.kind,
                    name: None,
                },
            )
        })
        .collect())
}

fn decode_schema_v2(bytes: &[u8]) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    let rows: Vec<(SchemaKey, SchemaEntryV2)> = decode_rkyv(bytes, "CORE/SCMA")?;
    Ok(rows
        .into_iter()
        .map(|(key, entry)| {
            (
                key,
                SchemaEntry {
                    kind: entry.kind,
                    name: entry.name,
                },
            )
        })
        .collect())
}

fn schema_wire_cmp<V>(lhs: &(SchemaKey, V), rhs: &(SchemaKey, V)) -> std::cmp::Ordering {
    (lhs.0.label.as_str(), lhs.0.property.as_str())
        .cmp(&(rhs.0.label.as_str(), rhs.0.property.as_str()))
}

pub(super) fn encode_composite_schemas(
    graph: &SeleneGraph,
) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(CompositeSchemaKey, CompositeSchemaEntry)> = graph
        .composite_property_index
        .iter()
        .map(|((label, _), entry)| {
            (
                CompositeSchemaKey {
                    label: *label,
                    properties: entry.declared_properties.iter().copied().collect(),
                },
                CompositeSchemaEntry {
                    kinds: entry.kinds().iter().copied().collect(),
                    name: entry.name,
                },
            )
        })
        .collect();
    rows.sort_by(composite_schema_wire_cmp);
    encode_rkyv(&rows, "CORE/CPIX")
}

pub(super) fn decode_composite_schemas(
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
        if !seen.insert((key.label, canonical)) {
            return Err(invalid_payload(format!(
                "CORE/CPIX rows contain duplicate composite registration for label {}",
                key.label
            )));
        }
    }
    Ok(())
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
