//! rkyv section payloads for the core graph provider.
//!
//! Section payload format: rkyv 0.8 archives over sorted `Vec<(K, Row)>`
//! intermediates per spec 04 §4.3 / decision D14. Per-row property bags
//! are postcard-encoded inside an `Arc<[u8]>` field on each archived row until
//! every stored `Value` variant has rkyv archivability.
//!
//! Compatibility note: per spec 04 §4.6, any format change to a CORE section
//! bumps the snapshot envelope version so older snapshots fail with
//! `PersistError::UnsupportedVersion` instead of being decoded as garbled
//! bytes. BRIEF-Item-4a STEP 9 exercises this: `CORE/NODE` / `CORE/EDGE` now
//! persist the explicit external `NodeId` / `EdgeId` per row (read from the
//! `row_to_id` column) instead of synthesizing `row + 1`, so a future
//! 4b-compacted snapshot whose ids != row+1 round-trips. That change bumped
//! `SNAPSHOT_VERSION_MINOR` 0 -> 1 (selene-persist); pre-STEP-9 (minor 0)
//! snapshots are cleanly rejected — a clean break, not a dual decoder
//! (deferred to 4c per the D14 amendment).
//! `CORE/VIDX` later added optional IVF construction config beside HNSW config,
//! which bumped `SNAPSHOT_VERSION_MINOR` 2 -> 3 for the same clean-break reason.
//!
//! `CORE/SCMA` schema rows are stored in memory in `(label, property)` order
//! via [`SchemaKey`]'s derived `Ord`, which is lexicographic through [`DbString`].
//! Their wire order is the same canonical lexicographic order by
//! `(label.as_str(), property.as_str())`; decode re-sorts defensively into that
//! canonical order before duplicate validation.

use std::collections::BTreeSet;
use std::sync::Arc;

use rkyv::{
    Place,
    rancor::Fallible,
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use selene_core::{
    DbString, EdgeId, HnswIndexConfig, IvfIndexConfig, LabelSet, NodeId, PropertyMap,
};
use serde::{Deserialize, Serialize};

use crate::core_provider::{inconsistent, invalid_payload};
use crate::graph::{GraphMeta, SeleneGraph};
use crate::typed_index::TypedIndexKind;
use crate::vector_index::{MAX_IVF_TARGET_CENTROIDS, VectorIndexKind};

mod codec;
mod gtyp;

// Generic section codec plumbing. Sibling child modules (`gtyp`) reach these
// through the `super::codec::` path directly; this `use` brings the names into
// scope for the section encoders/decoders in this file.
use codec::{
    decode_properties_blob, decode_rkyv, encode_properties_blob, encode_rkyv, validate_ids_unique,
    validate_sorted_unique,
};
// Re-exported to `core_provider` so the cap-boundary unit test can reach it.
pub(super) use codec::ensure_section_within_cap;
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
    pub label: DbString,
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
    label: DbString,
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
/// registrations. In-memory order follows local [`DbString`] handles; the
/// `CORE/SCMA` wire order is lexicographic by `label.as_str()` and
/// `property.as_str()` for cross-process stability.
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
    /// Node label the registration applies to.
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
/// fall-through — mirrors the `CORE/GTYP` collapse.
pub(super) const SCMA_VERSION: u8 = 2;

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
        // BRIEF-Item-4a STEP 9: persist the EXPLICIT external id from the
        // row_to_id column rather than synthesizing `row + 1`. Committed rows
        // (alive or deleted-but-kept under Option B) carry their real `NodeId`;
        // never-committed aborted-tx hole rows carry `NodeId::TOMBSTONE`, which
        // recovery skips (-> the id resolves NotFound, matching the live path).
        // This is the format change the SLSN minor-version bump guards: a future
        // 4b-compacted snapshot whose ids != row+1 round-trips because recovery
        // places rows by their stored position, not by `id - 1` arithmetic.
        let id = graph
            .node_store
            .row_to_id
            .get(row_index)
            .copied()
            .ok_or_else(|| {
                inconsistent(format!("node row_to_id column missing row {row_index}"))
            })?;
        rows.push((id, NodeArchiveRow::from_runtime(runtime, "CORE/NODE")?));
    }
    encode_rkyv(&rows, "CORE/NODE")
}

pub(super) fn decode_nodes(bytes: &[u8]) -> Result<Vec<(NodeId, NodeRow)>, crate::ProviderError> {
    let rows: Vec<(NodeId, NodeArchiveRow)> = decode_rkyv(bytes, "CORE/NODE")?;
    // BRIEF-Item-4a STEP 9: rows are no longer guaranteed sorted-ascending by id
    // (a 4b-compacted snapshot may store ids in any row order) and multiple
    // aborted-tx hole rows legitimately share `NodeId::TOMBSTONE`. Validate that
    // every *real* (non-tombstone) id is unique; row order is positional.
    validate_ids_unique(&rows, NodeId::TOMBSTONE, "CORE/NODE")?;
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
            label: label.clone(),
            source: *source,
            target: *target,
            properties: properties.clone(),
            alive: graph.edge_store.is_alive(row),
        };
        // BRIEF-Item-4a STEP 9: persist the explicit external id from the
        // row_to_id column (real `EdgeId`, or `EdgeId::TOMBSTONE` for a
        // never-committed hole row). See `encode_nodes` for the rationale.
        let id = graph
            .edge_store
            .row_to_id
            .get(row_index)
            .copied()
            .ok_or_else(|| {
                inconsistent(format!("edge row_to_id column missing row {row_index}"))
            })?;
        rows.push((id, EdgeArchiveRow::from_runtime(runtime, "CORE/EDGE")?));
    }
    encode_rkyv(&rows, "CORE/EDGE")
}

pub(super) fn decode_edges(bytes: &[u8]) -> Result<Vec<(EdgeId, EdgeRow)>, crate::ProviderError> {
    let rows: Vec<(EdgeId, EdgeArchiveRow)> = decode_rkyv(bytes, "CORE/EDGE")?;
    validate_ids_unique(&rows, EdgeId::TOMBSTONE, "CORE/EDGE")?;
    rows.into_iter()
        .map(|(id, row)| row.into_runtime("CORE/EDGE").map(|row| (id, row)))
        .collect()
}

pub(super) fn encode_schemas(graph: &SeleneGraph) -> Result<Vec<u8>, crate::ProviderError> {
    let mut rows: Vec<(SchemaKey, SchemaEntry)> = graph
        .property_index
        .iter()
        .map(|((label, property), entry)| {
            (
                SchemaKey {
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
    rows.sort_by(schema_wire_cmp);
    let mut payload = Vec::with_capacity(1);
    payload.push(SCMA_VERSION);
    payload.extend(encode_rkyv(&rows, "CORE/SCMA")?);
    ensure_section_within_cap("CORE/SCMA", payload.len())?;
    Ok(payload)
}

pub(super) fn decode_schemas(
    bytes: &[u8],
) -> Result<Vec<(SchemaKey, SchemaEntry)>, crate::ProviderError> {
    // Single-version clean break (greenfield, no shipped consumers): the leading
    // version byte must match `SCMA_VERSION`. A missing or mismatched byte is a
    // hard decode error — there is no legacy decoder to fall back to.
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

pub(super) fn encode_vector_schemas(graph: &SeleneGraph) -> Result<Vec<u8>, crate::ProviderError> {
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

pub(super) fn decode_vector_schemas(
    bytes: &[u8],
) -> Result<Vec<(VectorSchemaKey, VectorSchemaEntry)>, crate::ProviderError> {
    let mut rows: Vec<(VectorSchemaKey, VectorSchemaEntry)> = decode_rkyv(bytes, "CORE/VIDX")?;
    rows.sort_unstable_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
    validate_vector_schema_rows(&rows)?;
    Ok(rows)
}

pub(super) fn encode_text_schemas(graph: &SeleneGraph) -> Result<Vec<u8>, crate::ProviderError> {
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

pub(super) fn decode_text_schemas(
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
