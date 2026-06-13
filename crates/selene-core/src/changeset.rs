//! WAL change payloads per spec 02 section 9.
//!
//! The principal/audit actor lives in the WAL entry header per D12; these
//! payloads carry only the graph mutation itself. Diff payloads keep key lists
//! in canonical lexicographic order by [`DbString::as_str`] both in memory and on
//! the wire (the derived [`DbString`] `Ord` is lexicographic through the inner
//! string). Serialize canonicalizes (sorts) the lists before emitting — a no-op
//! for diffs built via the constructors, but load-bearing because the diff
//! fields are public and can be set non-canonically. Deserialize then validates
//! the canonical invariant and rejects a non-canonical or out-of-order payload
//! as malformed rather than re-sorting it.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::{
    CoreError, CoreResult, DbString, EdgeId, EdgeTypeDef, EdgeTypeDefV1, GraphId, GraphType,
    GraphTypeId, HnswIndexConfig, IvfIndexConfig, LabelSet, NodeId, NodeTypeDef, NodeTypeDefV1,
    PropertyMap, RecordTypeDef, Value,
};

/// A graph or schema change carried by the WAL.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
// Invariant: serde+postcard tag stability - append new variants, never insert.
// Reordering corrupts WAL files written under prior tag layouts.
pub enum Change {
    /// Node creation.
    NodeCreated {
        /// Created node ID.
        id: NodeId,
        /// Initial labels.
        labels: LabelSet,
        /// Initial properties.
        properties: PropertyMap,
    },
    /// Node update.
    NodeUpdated {
        /// Updated node ID.
        id: NodeId,
        /// Label changes.
        labels_diff: LabelDiff,
        /// Property changes.
        properties_diff: PropertyDiff,
    },
    /// Node deletion.
    NodeDeleted {
        /// Deleted node ID.
        id: NodeId,
    },
    /// Edge creation.
    EdgeCreated {
        /// Created edge ID.
        id: EdgeId,
        /// Edge label.
        label: DbString,
        /// Source node ID.
        source: NodeId,
        /// Target node ID.
        target: NodeId,
        /// Initial properties.
        properties: PropertyMap,
    },
    /// Edge update.
    EdgeUpdated {
        /// Updated edge ID.
        id: EdgeId,
        /// Property changes.
        properties_diff: PropertyDiff,
    },
    /// Edge deletion.
    EdgeDeleted {
        /// Deleted edge ID.
        id: EdgeId,
    },
    /// Schema mutation.
    SchemaChanged {
        /// Graph affected by the schema change.
        graph: GraphId,
        /// Schema change payload.
        change: SchemaChange,
    },
    /// Node property removal.
    NodePropertyRemoved {
        /// Updated node ID.
        id: NodeId,
        /// Removed property key.
        property: DbString,
    },
    /// Edge property removal.
    EdgePropertyRemoved {
        /// Updated edge ID.
        id: EdgeId,
        /// Removed property key.
        property: DbString,
    },
    /// Node label removal.
    NodeLabelRemoved {
        /// Updated node ID.
        id: NodeId,
        /// Removed label.
        label: DbString,
    },
    /// Bulk removal of every node carrying `label` plus all incident edges.
    ///
    /// This is the O(1)-WAL declarative truncate change (BRIEF-150, deletion-
    /// reclamation audit Item 11). It carries **only** the label — never the
    /// affected node/edge ids — so a `TRUNCATE NODE TYPE :L` of N nodes still
    /// writes exactly one WAL change. Recovery re-derives the affected rows by
    /// walking the recovered store ("replay walks store"), marking dead every
    /// alive node with `label` and every alive edge incident to such a node, so
    /// the recovered state is byte-identical to `MATCH (n:L) DETACH DELETE n`.
    /// Live commit fan-out substitutes the change with staged per-row
    /// `NodeDeleted`/`EdgeDeleted` tombstones when the mutator captured them
    /// during execution. WAL/recovery replay carries this persisted declarative
    /// variant, so provider-owned derived state must either handle it directly
    /// or rebuild from the recovered graph snapshot before serving reads.
    NodesOfTypeTruncated {
        /// Node label whose instances (and incident edges) were removed.
        label: DbString,
    },
    /// Bulk removal of every edge carrying `label`.
    ///
    /// The edge-type counterpart to [`Change::NodesOfTypeTruncated`]
    /// (`TRUNCATE EDGE TYPE :L`). Carries only the label (O(1) WAL); recovery
    /// re-derives the affected edges from the recovered store. Live commit
    /// fan-out substitutes the change with staged per-row `EdgeDeleted`
    /// tombstones when execution captured them; WAL/recovery replay carries
    /// this persisted declarative variant, so providers must handle it directly
    /// or rebuild before serving reads.
    EdgesOfTypeTruncated {
        /// Edge label whose instances were removed.
        label: DbString,
    },
    /// Factory-reset of the entire graph: wipe **all** nodes and edges (every
    /// label, including untyped/arbitrary-label rows) **and** reset the schema
    /// to open (`bound_type` -> `None`), in one declarative O(1)-WAL change.
    ///
    /// This is the `DROP GRAPH` factory-reset change (BRIEF-152, deletion-
    /// reclamation audit Item 10). Under D1 single-graph it targets the one
    /// bound graph. It carries **nothing** — never the affected node/edge ids
    /// nor any schema payload — so a reset of a graph with N rows still writes
    /// exactly one WAL change. Recovery re-derives every affected row by walking
    /// the recovered store ("replay walks store"), marking dead every alive node
    /// and edge, and forces the recovered `bound_type` to `None`, so the
    /// recovered state is byte-identical to `MATCH (n) DETACH DELETE n` followed
    /// by a full schema drop. Live commit fan-out substitutes the change with
    /// staged per-row `NodeDeleted`/`EdgeDeleted` tombstones when execution
    /// captured them. WAL/recovery replay carries this persisted declarative
    /// variant, so providers must handle it directly or rebuild before serving
    /// reads. The MANIFEST epoch and WAL archive lineage are untouched: a
    /// factory-reset is one committed WAL entry on top of the existing snapshot,
    /// not a file-level wipe.
    GraphReset {},
}

/// Label set difference.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelDiff {
    /// Labels added by the mutation.
    pub added: SmallVec<[DbString; 2]>,
    /// Labels removed by the mutation.
    pub removed: SmallVec<[DbString; 2]>,
}

impl LabelDiff {
    /// Construct a sorted, deduplicated label diff.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::OverlappingDiff`] when a label appears in both
    /// `added` and `removed`. Contradictory diffs would make WAL replay
    /// order-dependent, so the constructor refuses to build them.
    pub fn new(
        added: impl IntoIterator<Item = DbString>,
        removed: impl IntoIterator<Item = DbString>,
    ) -> CoreResult<Self> {
        let added = sorted_deduped(added);
        let removed = sorted_deduped(removed);
        ensure_disjoint("label", &added, &removed)?;
        Ok(Self { added, removed })
    }

    /// Return true if no labels changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Deserialize, Serialize)]
struct LabelDiffWire {
    added: SmallVec<[DbString; 2]>,
    removed: SmallVec<[DbString; 2]>,
}

impl Serialize for LabelDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Canonicalize on serialize. `LabelDiff::new` already sorts, so this is
        // a no-op (byte-identical) for constructed diffs — but `added`/`removed`
        // are public fields, so a caller can build a non-canonical diff directly;
        // sorting here guarantees the wire is canonical and round-trips through
        // the strict (validate, no-resort) deserializer below.
        let mut added = self.added.clone();
        let mut removed = self.removed.clone();
        added.sort_by(|lhs, rhs| lhs.as_str().cmp(rhs.as_str()));
        removed.sort_by(|lhs, rhs| lhs.as_str().cmp(rhs.as_str()));
        LabelDiffWire { added, removed }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LabelDiff {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Validate the canonical (strictly-ascending, dedup'd, disjoint)
        // invariant rather than re-sorting; a non-canonical payload is
        // rejected as malformed.
        let wire = LabelDiffWire::deserialize(deserializer)?;
        validate_sorted_unique(&wire.added, "LabelDiff.added")?;
        validate_sorted_unique(&wire.removed, "LabelDiff.removed")?;
        validate_disjoint(&wire.added, &wire.removed, "label")?;
        Ok(Self {
            added: wire.added,
            removed: wire.removed,
        })
    }
}

/// Property map difference.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDiff {
    /// Keys set to a new value. Use [`Value::Null`] for an explicit null set.
    pub set: SmallVec<[(DbString, Value); 4]>,
    /// Keys whose entries are removed entirely.
    pub removed: SmallVec<[DbString; 2]>,
}

impl PropertyDiff {
    /// Construct a sorted, deduplicated property diff.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::OverlappingDiff`] when a key appears in both `set`
    /// and `removed`. Contradictory diffs would make WAL replay
    /// order-dependent, so the constructor refuses to build them.
    pub fn new(
        set: impl IntoIterator<Item = (DbString, Value)>,
        removed: impl IntoIterator<Item = DbString>,
    ) -> CoreResult<Self> {
        let mut set: Vec<_> = set.into_iter().collect();
        set.sort_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
        set.dedup_by(|(lhs_key, lhs_value), (rhs_key, rhs_value)| {
            if lhs_key == rhs_key {
                *lhs_value = rhs_value.clone();
                true
            } else {
                false
            }
        });
        let set: SmallVec<[(DbString, Value); 4]> = set.into_iter().collect();
        let removed = sorted_deduped(removed);
        for (key, _) in set.iter() {
            if removed.binary_search(key).is_ok() {
                return Err(CoreError::OverlappingDiff {
                    kind: "property",
                    key: key.clone(),
                });
            }
        }
        Ok(Self { set, removed })
    }

    /// Return true if no properties changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.removed.is_empty()
    }
}

#[derive(Deserialize, Serialize)]
struct PropertyDiffWire {
    set: SmallVec<[(DbString, Value); 4]>,
    removed: SmallVec<[DbString; 2]>,
}

impl Serialize for PropertyDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Canonicalize on serialize. `PropertyDiff::new` already sorts, so this
        // is a no-op (byte-identical) for constructed diffs — but `set`/`removed`
        // are public fields, so a caller can build a non-canonical diff directly;
        // sorting here guarantees the wire is canonical and round-trips through
        // the strict (validate, no-resort) deserializer below.
        let mut set = self.set.clone();
        let mut removed = self.removed.clone();
        set.sort_by(|(lhs, _), (rhs, _)| lhs.as_str().cmp(rhs.as_str()));
        removed.sort_by(|lhs, rhs| lhs.as_str().cmp(rhs.as_str()));
        PropertyDiffWire { set, removed }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PropertyDiff {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Validate the canonical invariant (strictly-ascending set keys,
        // strictly-ascending removed, disjoint) rather than re-sorting; a
        // non-canonical payload is rejected as malformed.
        let wire = PropertyDiffWire::deserialize(deserializer)?;
        for window in wire.set.windows(2) {
            if window[0].0 >= window[1].0 {
                return Err(serde::de::Error::custom(
                    "PropertyDiff.set entries must be sorted by DbString order with no duplicate keys",
                ));
            }
        }
        validate_sorted_unique(&wire.removed, "PropertyDiff.removed")?;
        for (key, _) in wire.set.iter() {
            if wire.removed.binary_search(key).is_ok() {
                return Err(serde::de::Error::custom(format!(
                    "PropertyDiff: key {key} appears in both set and removed",
                )));
            }
        }
        Ok(Self {
            set: wire.set,
            removed: wire.removed,
        })
    }
}

/// Schema change payload.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum SchemaChange {
    /// Graph creation.
    GraphCreated {
        /// Created graph ID.
        id: GraphId,
        /// Graph name.
        name: DbString,
        /// Optional graph type assigned at creation.
        graph_type: Option<GraphTypeId>,
    },
    /// Graph deletion.
    GraphDropped {
        /// Dropped graph ID.
        id: GraphId,
    },
    /// Graph type creation.
    GraphTypeCreated {
        /// Created graph type definition.
        graph_type: GraphType,
    },
    /// Graph type deletion.
    GraphTypeDropped {
        /// Dropped graph type ID.
        id: GraphTypeId,
    },
    /// Node type addition.
    NodeTypeAdded {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Node type label.
        label: DbString,
        /// Legacy node type definition.
        def: NodeTypeDefV1,
    },
    /// Edge type addition.
    EdgeTypeAdded {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Edge type label.
        label: DbString,
        /// Legacy edge type definition.
        def: EdgeTypeDefV1,
    },
    /// Node type deletion.
    NodeTypeDropped {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Dropped node type name.
        name: DbString,
    },
    /// Edge type deletion.
    EdgeTypeDropped {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Dropped edge type name.
        name: DbString,
    },
    /// Record type addition.
    RecordTypeAdded {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Record type definition.
        def: RecordTypeDef,
    },
    /// Property index creation.
    PropertyIndexCreated {
        /// Indexed node label.
        label: DbString,
        /// Indexed property key.
        property: DbString,
        /// Declared index value kind.
        kind: SchemaPropertyIndexKind,
    },
    /// Property index deletion.
    PropertyIndexDropped {
        /// Indexed node label.
        label: DbString,
        /// Indexed property key.
        property: DbString,
    },
    /// Property index creation with optional explicit catalog name.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable. Old WALs continue
    /// to decode through [`SchemaChange::PropertyIndexCreated`].
    PropertyIndexCreatedNamed {
        /// Indexed node label.
        label: DbString,
        /// Indexed property key.
        property: DbString,
        /// Declared index value kind.
        kind: SchemaPropertyIndexKind,
        /// Optional explicit catalog name.
        name: Option<DbString>,
    },
    /// Node type addition carrying v2 type-model fields.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable. New code emits this
    /// variant; old WALs continue to decode through [`SchemaChange::NodeTypeAdded`].
    NodeTypeAddedV2 {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Node type label.
        label: DbString,
        /// Node type definition.
        def: NodeTypeDef,
    },
    /// Edge type addition carrying v2 type-model fields.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable. New code emits this
    /// variant; old WALs continue to decode through [`SchemaChange::EdgeTypeAdded`].
    EdgeTypeAddedV2 {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Edge type label.
        label: DbString,
        /// Edge type definition.
        def: EdgeTypeDef,
    },
    /// Composite property index creation with optional explicit catalog name.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    CompositePropertyIndexCreated {
        /// Indexed node label.
        label: DbString,
        /// Indexed property keys in declaration order.
        properties: SmallVec<[DbString; 4]>,
        /// Declared index value kinds in declaration order.
        kinds: SmallVec<[SchemaPropertyIndexKind; 4]>,
        /// Optional explicit catalog name.
        name: Option<DbString>,
    },
    /// Composite property index deletion.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    CompositePropertyIndexDropped {
        /// Indexed node label.
        label: DbString,
        /// Indexed property keys in declaration order.
        properties: SmallVec<[DbString; 4]>,
    },
    /// Vector property index creation with optional explicit catalog name.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    VectorIndexCreated {
        /// Indexed node label.
        label: DbString,
        /// Indexed vector property key.
        property: DbString,
        /// Declared vector index algorithm.
        kind: SchemaVectorIndexKind,
        /// Required vector dimensionality for indexed rows.
        dimension: u32,
        /// Optional explicit catalog name.
        name: Option<DbString>,
        /// Optional HNSW construction parameters.
        hnsw_config: Option<HnswIndexConfig>,
        /// Optional IVF construction parameters.
        ivf_config: Option<IvfIndexConfig>,
    },
    /// Vector property index deletion.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    VectorIndexDropped {
        /// Indexed node label.
        label: DbString,
        /// Indexed vector property key.
        property: DbString,
    },
    /// Text property index creation with optional explicit catalog name.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    TextIndexCreated {
        /// Indexed node label.
        label: DbString,
        /// Indexed string property key.
        property: DbString,
        /// Optional explicit catalog name.
        name: Option<DbString>,
    },
    /// Text property index deletion.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    TextIndexDropped {
        /// Indexed node label.
        label: DbString,
        /// Indexed string property key.
        property: DbString,
    },
}

/// Schema-level vector index algorithm kind.
///
/// This mirrors storage-level vector index algorithm selection without making
/// `selene-core` depend on graph storage internals.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SchemaVectorIndexKind {
    /// Exact in-memory row-set accelerator. ANN algorithms can be added as new
    /// variants without changing the `(label, property)` catalog identity.
    Flat,
    /// Approximate HNSW index using squared Euclidean distance.
    HnswSquaredEuclidean,
    /// Approximate HNSW index using cosine distance.
    HnswCosine,
    /// Approximate HNSW index using negative inner product distance.
    HnswNegativeInnerProduct,
    /// Approximate IVF index using squared Euclidean distance.
    IvfSquaredEuclidean,
    /// Approximate IVF index using cosine distance.
    IvfCosine,
    /// Approximate IVF index using negative inner product distance.
    IvfNegativeInnerProduct,
    /// Compressed TurboQuant candidate index using cosine distance.
    TurboQuantCosine,
}

/// Schema-level property index value kind.
///
/// This mirrors `selene_graph::TypedIndexKind` without making `selene-core`
/// depend on graph storage internals.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SchemaPropertyIndexKind {
    /// Boolean value.
    Bool,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 64-bit integer.
    U64,
    /// Signed 128-bit integer.
    I128,
    /// Unsigned 128-bit integer.
    U128,
    /// Fixed-precision decimal value.
    Decimal,
    /// Finite 32-bit floating-point value.
    F32,
    /// Finite 64-bit floating-point value.
    F64,
    /// Database string.
    String,
    /// Civil date.
    Date,
    /// Civil local date-time.
    LocalDateTime,
    /// Zoned date-time.
    ZonedDateTime,
    /// Civil local time.
    LocalTime,
    /// Zoned time.
    ZonedTime,
    /// Duration.
    Duration,
    /// UUID.
    Uuid,
}

fn sorted_deduped(values: impl IntoIterator<Item = DbString>) -> SmallVec<[DbString; 2]> {
    let mut values: SmallVec<[DbString; 2]> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values
}

fn ensure_disjoint(
    kind: &'static str,
    added: &SmallVec<[DbString; 2]>,
    removed: &SmallVec<[DbString; 2]>,
) -> CoreResult<()> {
    for label in added.iter() {
        if removed.binary_search(label).is_ok() {
            return Err(CoreError::OverlappingDiff {
                kind,
                key: label.clone(),
            });
        }
    }
    Ok(())
}

fn validate_sorted_unique<E: serde::de::Error>(
    values: &SmallVec<[DbString; 2]>,
    label: &'static str,
) -> Result<(), E> {
    for window in values.windows(2) {
        if window[0] >= window[1] {
            return Err(E::custom(format!(
                "{label} must be sorted by DbString order with no duplicates"
            )));
        }
    }
    Ok(())
}

fn validate_disjoint<E: serde::de::Error>(
    added: &SmallVec<[DbString; 2]>,
    removed: &SmallVec<[DbString; 2]>,
    kind: &'static str,
) -> Result<(), E> {
    for label in added.iter() {
        if removed.binary_search(label).is_ok() {
            return Err(E::custom(format!(
                "overlapping {kind} diff: {label} appears in both add/set and remove",
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "changeset/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "changeset/proptests.rs"]
mod proptests;
