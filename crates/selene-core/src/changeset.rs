//! WAL change payloads per spec 02 section 9.
//!
//! The principal/audit actor lives in the WAL entry header per D12; these
//! payloads carry only the graph mutation itself. Diff payloads keep
//! [`IStr`]-handle sorted storage in memory, but serialize key lists in
//! canonical lexicographic order by [`IStr::as_str`] and re-sort into the
//! receiver's local handle order after decode.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::{
    CoreError, CoreResult, EdgeId, EdgeTypeDef, GraphId, GraphType, GraphTypeId, IStr, LabelSet,
    NodeId, NodeTypeDef, NodeTypeRef, PackLifecycleEvent, PropertyMap, RecordTypeDef, RecordTypeId,
    Value,
};

/// A graph, schema, or extension-provider change carried by the WAL.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
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
    /// Opaque event emitted by an index extension provider.
    ///
    /// `provider` is a human-readable interned provider name (for example,
    /// `selene-vector`). The named provider owns deserialization of `payload`
    /// during WAL replay per D15.
    IndexExtensionEvent {
        /// Provider name.
        provider: IStr,
        /// Provider-owned payload bytes.
        payload: Arc<[u8]>,
    },
}

impl Change {
    /// Factory table with one sample change for each [`Change`] variant.
    ///
    /// Tests use this as an append-only ANCHOR so new WAL variants require a
    /// source-of-truth census update in `selene-core`.
    pub const ALL: &[fn() -> Self] = &[
        || Self::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::new(),
            properties: PropertyMap::new(),
        },
        || Self::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([], []).unwrap(),
            properties_diff: PropertyDiff::new([], []).unwrap(),
        },
        || Self::NodeDeleted { id: NodeId::new(1) },
        || Self::EdgeCreated {
            id: EdgeId::new(1),
            label: changeset_variant_istr("change.all.edge"),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
        || Self::EdgeUpdated {
            id: EdgeId::new(1),
            properties_diff: PropertyDiff::new([], []).unwrap(),
        },
        || Self::EdgeDeleted { id: EdgeId::new(1) },
        || Self::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::GraphDropped {
                id: GraphId::new(2),
            },
        },
        || Self::IndexExtensionEvent {
            provider: changeset_variant_istr("change.all.provider"),
            payload: Arc::from([0_u8]),
        },
    ];

    /// Number of known [`Change`] variants in this build.
    pub const VARIANT_COUNT: usize = Self::ALL.len();

    /// Stable telemetry name for this change variant.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::NodeCreated { .. } => "NodeCreated",
            Self::NodeUpdated { .. } => "NodeUpdated",
            Self::NodeDeleted { .. } => "NodeDeleted",
            Self::EdgeCreated { .. } => "EdgeCreated",
            Self::EdgeUpdated { .. } => "EdgeUpdated",
            Self::EdgeDeleted { .. } => "EdgeDeleted",
            Self::SchemaChanged { .. } => "SchemaChanged",
            Self::IndexExtensionEvent { .. } => "IndexExtensionEvent",
        }
    }
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
        wire.added.sort_unstable_by_key(|key| *key);
        wire.removed.sort_unstable_by_key(|key| *key);
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
        set.sort_by_key(|(key, _)| *key);
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
                    key: *key,
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
        wire.set.sort_unstable_by_key(|(key, _)| *key);
        wire.removed.sort_unstable_by_key(|key| *key);
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
        /// Node type definition.
        def: NodeTypeDef,
    },
    /// Edge type addition.
    EdgeTypeAdded {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Edge type label.
        label: IStr,
        /// Edge type definition.
        def: EdgeTypeDef,
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
    /// Reserved — legacy procedure-pack activation placeholder.
    ///
    /// Retained at this position so the `postcard` discriminant of every
    /// subsequent variant stays stable. No selene-db code emits this variant;
    /// recovery does not act on it. New code emits
    /// [`SchemaChange::ProcedurePackLifecycle`] instead.
    #[doc(hidden)]
    ProcedurePackActivated {
        /// Procedure pack name.
        pack_name: IStr,
        /// Procedure pack version.
        version: IStr,
    },
    /// Reserved — legacy procedure-pack deprecation placeholder.
    ///
    /// Retained for `postcard` ABI stability (see
    /// [`SchemaChange::ProcedurePackActivated`]). No selene-db code emits or
    /// applies this variant.
    #[doc(hidden)]
    ProcedurePackDeprecated {
        /// Procedure pack name.
        pack_name: IStr,
        /// Procedure pack version.
        version: IStr,
        /// Interned short reason.
        reason: IStr,
    },
    /// Reserved — legacy procedure-pack disable placeholder.
    ///
    /// Retained for `postcard` ABI stability (see
    /// [`SchemaChange::ProcedurePackActivated`]). No selene-db code emits or
    /// applies this variant.
    #[doc(hidden)]
    ProcedurePackDisabled {
        /// Procedure pack name.
        pack_name: IStr,
        /// Procedure pack version.
        version: IStr,
        /// Interned short reason.
        reason: IStr,
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
    /// Procedure-pack lifecycle audit event.
    ///
    /// Declared after [`SchemaChange::PropertyIndexDropped`] so the
    /// `postcard` discriminants of all earlier variants remain stable across
    /// BRIEF-46. The legacy `ProcedurePack*` variants above this entry are
    /// retained but never emitted; new code emits `ProcedurePackLifecycle`.
    ProcedurePackLifecycle {
        /// Pack lifecycle event payload.
        event: PackLifecycleEvent,
    },
}

impl SchemaChange {
    /// Factory table with one sample change for each [`SchemaChange`] variant.
    ///
    /// Hidden legacy variants are included because their postcard
    /// discriminants are reserved for WAL compatibility.
    pub const ALL: &[fn() -> Self] = &[
        || Self::GraphCreated {
            id: GraphId::new(1),
            name: changeset_variant_istr("schema.all.graph"),
            graph_type: Some(changeset_graph_type_id()),
        },
        || Self::GraphDropped {
            id: GraphId::new(1),
        },
        || Self::GraphTypeCreated {
            graph_type: changeset_graph_type(),
        },
        || Self::GraphTypeDropped {
            id: changeset_graph_type_id(),
        },
        || Self::NodeTypeAdded {
            graph_type: changeset_graph_type_id(),
            label: changeset_variant_istr("schema.all.node"),
            def: NodeTypeDef::new(LabelSet::single(changeset_variant_istr("schema.all.node"))),
        },
        || Self::EdgeTypeAdded {
            graph_type: changeset_graph_type_id(),
            label: changeset_variant_istr("schema.all.edge"),
            def: EdgeTypeDef::new(
                changeset_variant_istr("schema.all.edge"),
                NodeTypeRef(changeset_variant_istr("schema.all.node")),
                NodeTypeRef(changeset_variant_istr("schema.all.node")),
            ),
        },
        || Self::NodeTypeDropped {
            graph_type: changeset_graph_type_id(),
            name: changeset_variant_istr("schema.all.node"),
        },
        || Self::EdgeTypeDropped {
            graph_type: changeset_graph_type_id(),
            name: changeset_variant_istr("schema.all.edge"),
        },
        || Self::RecordTypeAdded {
            graph_type: changeset_graph_type_id(),
            def: RecordTypeDef {
                id: RecordTypeId::new(1),
                name: changeset_variant_istr("schema.all.record"),
                fields: SmallVec::new(),
            },
        },
        || Self::ProcedurePackActivated {
            pack_name: changeset_variant_istr("schema.all.pack"),
            version: changeset_variant_istr("schema.all.version"),
        },
        || Self::ProcedurePackDeprecated {
            pack_name: changeset_variant_istr("schema.all.pack"),
            version: changeset_variant_istr("schema.all.version"),
            reason: changeset_variant_istr("schema.all.reason"),
        },
        || Self::ProcedurePackDisabled {
            pack_name: changeset_variant_istr("schema.all.pack"),
            version: changeset_variant_istr("schema.all.version"),
            reason: changeset_variant_istr("schema.all.reason"),
        },
        || Self::PropertyIndexCreated {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.property"),
            kind: SchemaPropertyIndexKind::I64,
        },
        || Self::PropertyIndexDropped {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.property"),
        },
        || Self::ProcedurePackLifecycle {
            event: PackLifecycleEvent::ValidationFailed {
                pack_name: Some(changeset_variant_istr("schema.all.pack")),
                principal: changeset_variant_istr("schema.all.principal"),
                error: "schema.all.error".to_owned(),
                at: changeset_timestamp(1),
            },
        },
    ];

    /// Number of known [`SchemaChange`] variants in this build.
    pub const VARIANT_COUNT: usize = Self::ALL.len();

    /// Stable telemetry name for this schema-change variant.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::GraphCreated { .. } => "GraphCreated",
            Self::GraphDropped { .. } => "GraphDropped",
            Self::GraphTypeCreated { .. } => "GraphTypeCreated",
            Self::GraphTypeDropped { .. } => "GraphTypeDropped",
            Self::NodeTypeAdded { .. } => "NodeTypeAdded",
            Self::EdgeTypeAdded { .. } => "EdgeTypeAdded",
            Self::NodeTypeDropped { .. } => "NodeTypeDropped",
            Self::EdgeTypeDropped { .. } => "EdgeTypeDropped",
            Self::RecordTypeAdded { .. } => "RecordTypeAdded",
            Self::ProcedurePackActivated { .. } => "ProcedurePackActivated",
            Self::ProcedurePackDeprecated { .. } => "ProcedurePackDeprecated",
            Self::ProcedurePackDisabled { .. } => "ProcedurePackDisabled",
            Self::PropertyIndexCreated { .. } => "PropertyIndexCreated",
            Self::PropertyIndexDropped { .. } => "PropertyIndexDropped",
            Self::ProcedurePackLifecycle { .. } => "ProcedurePackLifecycle",
        }
    }
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

fn changeset_variant_istr(name: &str) -> IStr {
    crate::intern(name).expect("Change::ALL fixture strings fit the process interner cap")
}

fn changeset_graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).expect("Change::ALL graph type fixture is non-zero")
}

fn changeset_graph_type() -> GraphType {
    GraphType::new(
        changeset_graph_type_id(),
        changeset_variant_istr("schema.all.graph_type"),
    )
}

fn changeset_timestamp(second: i64) -> jiff::Timestamp {
    jiff::Timestamp::new(second, 0).expect("Change::ALL timestamp fixture is in range")
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
            return Err(CoreError::OverlappingDiff { kind, key: *label });
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
    fn index_extension_event_payload_round_trip() {
        let change = Change::IndexExtensionEvent {
            provider: istr("selene-vector"),
            payload: Arc::from([1_u8, 2, 3]),
        };
        assert_eq!(change.clone(), change);
    }

    #[test]
    fn change_all_covers_every_variant() {
        assert_eq!(Change::VARIANT_COUNT, 8);
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
        let diff = LabelDiff::new([added], [removed]).unwrap();
        assert_eq!(diff.added.as_slice(), &[added]);
        assert_eq!(diff.removed.as_slice(), &[removed]);
    }

    #[test]
    fn property_diff_set_includes_null_value() {
        let property = istr("change.null");
        let diff = PropertyDiff::new([(property, Value::Null)], []).unwrap();
        assert_eq!(diff.set.as_slice(), &[(property, Value::Null)]);
    }

    #[test]
    fn label_diff_rejects_overlapping_label() {
        let label = istr("change.overlap.label");
        let err = LabelDiff::new([label], [label]).unwrap_err();
        assert!(matches!(
            err,
            CoreError::OverlappingDiff { kind: "label", .. }
        ));
    }

    #[test]
    fn property_diff_rejects_overlapping_key() {
        let key = istr("change.overlap.prop");
        let err = PropertyDiff::new([(key, Value::Int(1))], [key]).unwrap_err();
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
    fn label_diff_deserialize_resorts_by_receiver_handle() {
        let b = istr("change.deser.label.zebra");
        let a = istr("change.deser.label.apple");
        let bad = LabelDiffWireSer {
            added: smallvec![a, b],
            removed: SmallVec::new(),
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let round: LabelDiff = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(round.added, SmallVec::<[IStr; 2]>::from_vec(vec![b, a]));
    }

    #[test]
    fn label_diff_deserialize_rejects_duplicate_added() {
        let label = istr("change.deser.label.dup");
        let bad = LabelDiffWireSer {
            added: smallvec![label, label],
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
        added.push(label);
        let mut removed = SmallVec::<[IStr; 2]>::new();
        removed.push(label);
        let bad = LabelDiffWireSer { added, removed };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let result: Result<LabelDiff, _> = postcard::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn property_diff_deserialize_resorts_by_receiver_handle() {
        let b = istr("change.deser.prop.zebra");
        let a = istr("change.deser.prop.apple");
        let bad = PropertyDiffWireSer {
            set: smallvec![(a, Value::Int(1)), (b, Value::Int(2))],
            removed: SmallVec::new(),
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let round: PropertyDiff = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(
            round.set,
            SmallVec::<[(IStr, Value); 4]>::from_vec(vec![(b, Value::Int(2)), (a, Value::Int(1)),])
        );
    }

    #[test]
    fn property_diff_deserialize_rejects_duplicate_set_key() {
        let key = istr("change.deser.prop.dup");
        let bad = PropertyDiffWireSer {
            set: smallvec![(key, Value::Int(1)), (key, Value::Int(2))],
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
        set.push((key, Value::Int(1)));
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
    fn schema_change_procedure_pack_lifecycle() {
        let name = istr("pack");
        let reason = istr("retired");
        let staged = SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Staged {
                pack_name: name,
                content_hash: [0_u8; 32],
                principal: istr("principal"),
                at: jiff::Timestamp::new(1, 0).unwrap(),
            },
        };
        let deprecated = SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Deprecated {
                pack_name: name,
                reason,
                principal: istr("principal"),
                at: jiff::Timestamp::new(2, 0).unwrap(),
            },
        };
        let disabled = SchemaChange::ProcedurePackLifecycle {
            event: PackLifecycleEvent::Disabled {
                pack_name: name,
                principal: istr("principal"),
                at: jiff::Timestamp::new(3, 0).unwrap(),
            },
        };
        assert_ne!(staged, deprecated);
        assert_ne!(deprecated, disabled);
    }

    #[test]
    fn empty_diffs_and_empty_payload_are_valid() {
        assert!(LabelDiff::new([], []).unwrap().is_empty());
        assert!(PropertyDiff::new([], []).unwrap().is_empty());
        let event = Change::IndexExtensionEvent {
            provider: istr("empty-provider"),
            payload: Arc::from([]),
        };
        assert_eq!(event.clone(), event);
    }

    #[test]
    fn schema_change_variants_construct() {
        let variants: Vec<_> = SchemaChange::ALL.iter().map(|factory| factory()).collect();
        assert_eq!(variants.len(), SchemaChange::VARIANT_COUNT);
        assert_eq!(SchemaChange::VARIANT_COUNT, 15);
    }

    #[test]
    fn schema_change_all_covers_every_variant() {
        assert_eq!(SchemaChange::VARIANT_COUNT, 15);
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
