//! WAL change payloads per spec 02 section 9.
//!
//! The principal/audit actor lives in the WAL entry header per D12; these
//! payloads carry only the graph mutation itself. Diff payloads keep key lists
//! in canonical lexicographic order by [`IStr::as_str`] both in memory and on
//! the wire (the derived [`IStr`] `Ord` is lexicographic through the inner
//! string), and re-sort defensively into that canonical order after decode so
//! a hand-built or out-of-order wire payload is canonicalized on receipt.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::{
    CoreError, CoreResult, EdgeId, EdgeTypeDef, EdgeTypeDefV1, GraphId, GraphType, GraphTypeId,
    IStr, LabelSet, NodeId, NodeTypeDef, NodeTypeDefV1, PropertyMap, RecordTypeDef, Value,
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
        label: IStr,
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
        property: IStr,
    },
    /// Edge property removal.
    EdgePropertyRemoved {
        /// Updated edge ID.
        id: EdgeId,
        /// Removed property key.
        property: IStr,
    },
    /// Node label removal.
    NodeLabelRemoved {
        /// Updated node ID.
        id: NodeId,
        /// Removed label.
        label: IStr,
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
    /// Derived-state index providers never receive this declarative variant; the
    /// producing side expands it into per-row `NodeDeleted`/`EdgeDeleted`
    /// tombstones on both the runtime and recovery paths so derived state is
    /// reclaimed without leaks.
    NodesOfTypeTruncated {
        /// Node label whose instances (and incident edges) were removed.
        label: IStr,
    },
    /// Bulk removal of every edge carrying `label`.
    ///
    /// The edge-type counterpart to [`Change::NodesOfTypeTruncated`]
    /// (`TRUNCATE EDGE TYPE :L`). Carries only the label (O(1) WAL); recovery
    /// re-derives the affected edges from the recovered store. Index providers
    /// receive per-row `EdgeDeleted` tombstones, never this declarative variant.
    EdgesOfTypeTruncated {
        /// Edge label whose instances were removed.
        label: IStr,
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
    /// by a full schema drop. Derived-state index providers never receive this
    /// declarative variant; the producing side expands it into per-row
    /// `NodeDeleted`/`EdgeDeleted` tombstones on both the runtime and recovery
    /// paths so derived state is reclaimed without leaks. The MANIFEST epoch and
    /// WAL archive lineage are untouched: a factory-reset is one committed WAL
    /// entry on top of the existing snapshot, not a file-level wipe.
    GraphReset {},
}

/// Label set difference.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelDiff {
    /// Labels added by the mutation.
    pub added: SmallVec<[IStr; 2]>,
    /// Labels removed by the mutation.
    pub removed: SmallVec<[IStr; 2]>,
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
        added: impl IntoIterator<Item = IStr>,
        removed: impl IntoIterator<Item = IStr>,
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
    added: SmallVec<[IStr; 2]>,
    removed: SmallVec<[IStr; 2]>,
}

impl Serialize for LabelDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
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
        let mut wire = LabelDiffWire::deserialize(deserializer)?;
        wire.added.sort_unstable();
        wire.removed.sort_unstable();
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
    pub set: SmallVec<[(IStr, Value); 4]>,
    /// Keys whose entries are removed entirely.
    pub removed: SmallVec<[IStr; 2]>,
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
        set: impl IntoIterator<Item = (IStr, Value)>,
        removed: impl IntoIterator<Item = IStr>,
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
        let set: SmallVec<[(IStr, Value); 4]> = set.into_iter().collect();
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
    set: SmallVec<[(IStr, Value); 4]>,
    removed: SmallVec<[IStr; 2]>,
}

impl Serialize for PropertyDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
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
        let mut wire = PropertyDiffWire::deserialize(deserializer)?;
        wire.set.sort_unstable_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));
        wire.removed.sort_unstable();
        for window in wire.set.windows(2) {
            if window[0].0 >= window[1].0 {
                return Err(serde::de::Error::custom(
                    "PropertyDiff.set entries have duplicate keys",
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
        name: IStr,
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
        label: IStr,
        /// Legacy node type definition.
        def: NodeTypeDefV1,
    },
    /// Edge type addition.
    EdgeTypeAdded {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Edge type label.
        label: IStr,
        /// Legacy edge type definition.
        def: EdgeTypeDefV1,
    },
    /// Node type deletion.
    NodeTypeDropped {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Dropped node type name.
        name: IStr,
    },
    /// Edge type deletion.
    EdgeTypeDropped {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Dropped edge type name.
        name: IStr,
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
        label: IStr,
        /// Indexed property key.
        property: IStr,
        /// Declared index value kind.
        kind: SchemaPropertyIndexKind,
    },
    /// Property index deletion.
    PropertyIndexDropped {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
    },
    /// Property index creation with optional explicit catalog name.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable. Old WALs continue
    /// to decode through [`SchemaChange::PropertyIndexCreated`].
    PropertyIndexCreatedNamed {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
        /// Declared index value kind.
        kind: SchemaPropertyIndexKind,
        /// Optional explicit catalog name.
        name: Option<IStr>,
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
        label: IStr,
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
        label: IStr,
        /// Edge type definition.
        def: EdgeTypeDef,
    },
    /// Composite property index creation with optional explicit catalog name.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    CompositePropertyIndexCreated {
        /// Indexed node label.
        label: IStr,
        /// Indexed property keys in declaration order.
        properties: SmallVec<[IStr; 4]>,
        /// Declared index value kinds in declaration order.
        kinds: SmallVec<[SchemaPropertyIndexKind; 4]>,
        /// Optional explicit catalog name.
        name: Option<IStr>,
    },
    /// Composite property index deletion.
    ///
    /// Declared after every existing v1.1 variant so the `postcard`
    /// discriminants of all earlier variants remain stable.
    CompositePropertyIndexDropped {
        /// Indexed node label.
        label: IStr,
        /// Indexed property keys in declaration order.
        properties: SmallVec<[IStr; 4]>,
    },
}

/// Schema-level property index value kind.
///
/// This mirrors `selene_graph::TypedIndexKind` without making `selene-core`
/// depend on graph storage internals.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SchemaPropertyIndexKind {
    /// Signed 64-bit integer.
    I64,
    /// Finite 64-bit floating-point value.
    F64,
    /// Interned string.
    String,
    /// Civil date.
    Date,
    /// Civil local date-time.
    LocalDateTime,
    /// UUID.
    Uuid,
}

fn sorted_deduped(values: impl IntoIterator<Item = IStr>) -> SmallVec<[IStr; 2]> {
    let mut values: SmallVec<[IStr; 2]> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values
}

fn ensure_disjoint(
    kind: &'static str,
    added: &SmallVec<[IStr; 2]>,
    removed: &SmallVec<[IStr; 2]>,
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
    values: &SmallVec<[IStr; 2]>,
    label: &'static str,
) -> Result<(), E> {
    for window in values.windows(2) {
        if window[0] >= window[1] {
            return Err(E::custom(format!(
                "{label} must be sorted by IStr order with no duplicates"
            )));
        }
    }
    Ok(())
}

fn validate_disjoint<E: serde::de::Error>(
    added: &SmallVec<[IStr; 2]>,
    removed: &SmallVec<[IStr; 2]>,
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
mod tests {
    use proptest::prelude::*;
    use smallvec::smallvec;

    use super::*;
    use crate::{GraphTypeId, intern};

    fn istr(name: &str) -> IStr {
        intern(name).unwrap()
    }

    #[test]
    fn node_created_round_trip() {
        let change = Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(istr("change.node")),
            properties: PropertyMap::from_pairs([(istr("change.p"), Value::Int(1))]).unwrap(),
        };
        assert_eq!(change.clone(), change);
    }

    #[test]
    fn node_updated_with_label_diff_and_property_diff() {
        let change = Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([istr("change.add")], [istr("change.remove")]).unwrap(),
            properties_diff: PropertyDiff::new([(istr("change.set"), Value::Bool(true))], [])
                .unwrap(),
        };
        assert_eq!(change.clone(), change);
    }

    #[test]
    fn edge_lifecycle_create_update_delete() {
        let create = Change::EdgeCreated {
            id: EdgeId::new(1),
            label: istr("change.edge"),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        };
        let update = Change::EdgeUpdated {
            id: EdgeId::new(1),
            properties_diff: PropertyDiff::new([], [istr("change.removed")]).unwrap(),
        };
        let delete = Change::EdgeDeleted { id: EdgeId::new(1) };
        assert_ne!(create, update);
        assert_ne!(update, delete);
    }

    #[test]
    fn schema_changed_carries_graph_id_and_change_kind() {
        let graph_type = GraphTypeId::new(1).unwrap();
        let change = Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::GraphCreated {
                id: GraphId::new(2),
                name: istr("change.graph"),
                graph_type: Some(graph_type),
            },
        };
        match change {
            Change::SchemaChanged { graph, .. } => assert_eq!(graph, GraphId::new(1)),
            _ => panic!("expected schema change"),
        }
    }

    #[test]
    fn change_all_covers_every_variant() {
        assert_eq!(Change::VARIANT_COUNT, 13);
        let mut discriminants = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for factory in Change::ALL {
            let change = factory();
            assert!(
                discriminants.insert(std::mem::discriminant(&change)),
                "Change::ALL has duplicate variant: {}",
                change.variant_name()
            );
            let name = change.variant_name();
            assert!(!name.is_empty(), "Change::variant_name must not be empty");
            assert!(names.insert(name), "Change::variant_name collision: {name}");
        }
        assert_eq!(discriminants.len(), Change::ALL.len());
        assert_eq!(names.len(), Change::ALL.len());
    }

    #[test]
    fn label_diff_added_and_removed_independent() {
        let added = istr("change.label.added");
        let removed = istr("change.label.removed");
        let diff = LabelDiff::new([added.clone()], [removed.clone()]).unwrap();
        assert_eq!(diff.added.as_slice(), &[added]);
        assert_eq!(diff.removed.as_slice(), &[removed]);
    }

    #[test]
    fn property_diff_set_includes_null_value() {
        let property = istr("change.null");
        let diff = PropertyDiff::new([(property.clone(), Value::Null)], []).unwrap();
        assert_eq!(diff.set.as_slice(), &[(property, Value::Null)]);
    }

    #[test]
    fn label_diff_rejects_overlapping_label() {
        let label = istr("change.overlap.label");
        let err = LabelDiff::new([label.clone()], [label]).unwrap_err();
        assert!(matches!(
            err,
            CoreError::OverlappingDiff { kind: "label", .. }
        ));
    }

    #[test]
    fn property_diff_rejects_overlapping_key() {
        let key = istr("change.overlap.prop");
        let err = PropertyDiff::new([(key.clone(), Value::Int(1))], [key]).unwrap_err();
        assert!(matches!(
            err,
            CoreError::OverlappingDiff {
                kind: "property",
                ..
            }
        ));
    }

    #[test]
    fn label_diff_deserialize_round_trip() {
        let added = istr("change.deser.add");
        let removed = istr("change.deser.remove");
        let diff = LabelDiff::new([added], [removed]).unwrap();
        let bytes = postcard::to_allocvec(&diff).unwrap();
        let round: LabelDiff = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(round, diff);
    }

    #[test]
    fn label_diff_deserialize_resorts_lexicographically() {
        // Hand-build a wire payload whose key list is *not* in canonical
        // lexicographic order; the decoder must canonicalize it. `IStr` Ord is
        // lexicographic through the inner string, so "apple" sorts before
        // "zebra" regardless of intern order.
        let zebra = istr("change.deser.label.zebra");
        let apple = istr("change.deser.label.apple");
        let bad = LabelDiffWireSer {
            added: smallvec![zebra.clone(), apple.clone()],
            removed: SmallVec::new(),
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let round: LabelDiff = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(
            round.added,
            SmallVec::<[IStr; 2]>::from_vec(vec![apple, zebra])
        );
    }

    #[test]
    fn label_diff_deserialize_rejects_duplicate_added() {
        let label = istr("change.deser.label.dup");
        let bad = LabelDiffWireSer {
            added: smallvec![label.clone(), label],
            removed: SmallVec::new(),
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let result: Result<LabelDiff, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn label_diff_deserialize_rejects_overlap() {
        let label = istr("change.deser.bad");
        let mut added = SmallVec::<[IStr; 2]>::new();
        added.push(label.clone());
        let mut removed = SmallVec::<[IStr; 2]>::new();
        removed.push(label);
        let bad = LabelDiffWireSer { added, removed };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let result: Result<LabelDiff, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn property_diff_deserialize_resorts_lexicographically() {
        // As above for the property-set key list: an out-of-order wire payload
        // is canonicalized to lexicographic key order on decode.
        let zebra = istr("change.deser.prop.zebra");
        let apple = istr("change.deser.prop.apple");
        let bad = PropertyDiffWireSer {
            set: smallvec![
                (zebra.clone(), Value::Int(2)),
                (apple.clone(), Value::Int(1))
            ],
            removed: SmallVec::new(),
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let round: PropertyDiff = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(
            round.set,
            SmallVec::<[(IStr, Value); 4]>::from_vec(vec![
                (apple, Value::Int(1)),
                (zebra, Value::Int(2)),
            ])
        );
    }

    #[test]
    fn property_diff_deserialize_rejects_duplicate_set_key() {
        let key = istr("change.deser.prop.dup");
        let bad = PropertyDiffWireSer {
            set: smallvec![(key.clone(), Value::Int(1)), (key, Value::Int(2))],
            removed: SmallVec::new(),
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let result: Result<PropertyDiff, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn property_diff_deserialize_rejects_overlap() {
        let key = istr("change.deser.prop");
        let mut set = SmallVec::<[(IStr, Value); 4]>::new();
        set.push((key.clone(), Value::Int(1)));
        let mut removed = SmallVec::<[IStr; 2]>::new();
        removed.push(key);
        let bad = PropertyDiffWireSer { set, removed };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let result: Result<PropertyDiff, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[derive(serde::Serialize)]
    struct LabelDiffWireSer {
        added: SmallVec<[IStr; 2]>,
        removed: SmallVec<[IStr; 2]>,
    }

    #[derive(serde::Serialize)]
    struct PropertyDiffWireSer {
        set: SmallVec<[(IStr, Value); 4]>,
        removed: SmallVec<[IStr; 2]>,
    }

    #[test]
    fn empty_diffs_are_valid() {
        assert!(LabelDiff::new([], []).unwrap().is_empty());
        assert!(PropertyDiff::new([], []).unwrap().is_empty());
    }

    #[test]
    fn schema_change_variants_construct() {
        let variants: Vec<_> = SchemaChange::ALL.iter().map(|factory| factory()).collect();
        assert_eq!(variants.len(), SchemaChange::VARIANT_COUNT);
        assert_eq!(SchemaChange::VARIANT_COUNT, 16);
    }

    #[test]
    fn schema_change_all_covers_every_variant() {
        assert_eq!(SchemaChange::VARIANT_COUNT, 16);
        let mut discriminants = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for factory in SchemaChange::ALL {
            let change = factory();
            assert!(
                discriminants.insert(std::mem::discriminant(&change)),
                "SchemaChange::ALL has duplicate variant: {}",
                change.variant_name()
            );
            let name = change.variant_name();
            assert!(
                !name.is_empty(),
                "SchemaChange::variant_name must not be empty"
            );
            assert!(
                names.insert(name),
                "SchemaChange::variant_name collision: {name}"
            );
        }
        assert_eq!(discriminants.len(), SchemaChange::ALL.len());
        assert_eq!(names.len(), SchemaChange::ALL.len());
    }

    proptest! {
        #[test]
        fn random_label_diff_preserves_sorted_deduped(raw_added in proptest::collection::vec(0_u8..32, 0..32), raw_removed in proptest::collection::vec(33_u8..64, 0..32)) {
            let added = raw_added.into_iter().map(|value| {
                let name = format!("change.diff.{value}");
                intern(&name).unwrap()
            });
            let removed = raw_removed.into_iter().map(|value| {
                let name = format!("change.diff.{value}");
                intern(&name).unwrap()
            });
            let diff = LabelDiff::new(added, removed).unwrap();
            prop_assert!(diff.added.windows(2).all(|pair| pair[0] < pair[1]));
            prop_assert!(diff.removed.windows(2).all(|pair| pair[0] < pair[1]));
            prop_assert!(diff.added.iter().all(|label| !diff.removed.contains(label)));
        }

        #[test]
        fn random_property_diff_preserves_sorted_sets(raw_set in proptest::collection::vec(0_u8..32, 0..32), raw_removed in proptest::collection::vec(33_u8..64, 0..32)) {
            let set = raw_set.into_iter().map(|value| {
                let name = format!("change.prop.{value}");
                (intern(&name).unwrap(), Value::Uint(u64::from(value)))
            });
            let removed = raw_removed.into_iter().map(|value| {
                let name = format!("change.prop.{value}");
                intern(&name).unwrap()
            });
            let diff = PropertyDiff::new(set, removed).unwrap();
            prop_assert!(diff.set.windows(2).all(|pair| pair[0].0 < pair[1].0));
            prop_assert!(diff.removed.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}
