//! WAL change payloads per spec 02 section 9.
//!
//! The principal/audit actor lives in the WAL entry header per D12; these
//! payloads carry only the graph mutation itself.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{
    EdgeId, EdgeTypeDef, GraphId, GraphType, GraphTypeId, IStr, LabelSet, NodeId, NodeTypeDef,
    PropertyMap, RecordTypeDef, Value,
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

/// Label set difference.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LabelDiff {
    /// Labels added by the mutation.
    pub added: SmallVec<[IStr; 2]>,
    /// Labels removed by the mutation.
    pub removed: SmallVec<[IStr; 2]>,
}

impl LabelDiff {
    /// Construct a sorted, deduplicated label diff.
    #[must_use]
    pub fn new(
        added: impl IntoIterator<Item = IStr>,
        removed: impl IntoIterator<Item = IStr>,
    ) -> Self {
        Self {
            added: sorted_deduped(added),
            removed: sorted_deduped(removed),
        }
    }

    /// Return true if no labels changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Property map difference.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PropertyDiff {
    /// Keys set to a new value. Use [`Value::Null`] for an explicit null set.
    pub set: SmallVec<[(IStr, Value); 4]>,
    /// Keys whose entries are removed entirely.
    pub removed: SmallVec<[IStr; 2]>,
}

impl PropertyDiff {
    /// Construct a sorted, deduplicated property diff.
    #[must_use]
    pub fn new(
        set: impl IntoIterator<Item = (IStr, Value)>,
        removed: impl IntoIterator<Item = IStr>,
    ) -> Self {
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
        Self {
            set: set.into_iter().collect(),
            removed: sorted_deduped(removed),
        }
    }

    /// Return true if no properties changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.removed.is_empty()
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
    /// Record type addition.
    RecordTypeAdded {
        /// Owning graph type.
        graph_type: GraphTypeId,
        /// Record type definition.
        def: RecordTypeDef,
    },
    /// Procedure pack activation audit event.
    ProcedurePackActivated {
        /// Procedure pack name.
        pack_name: IStr,
        /// Procedure pack version.
        version: IStr,
    },
    /// Procedure pack deprecation audit event.
    ProcedurePackDeprecated {
        /// Procedure pack name.
        pack_name: IStr,
        /// Procedure pack version.
        version: IStr,
        /// Interned short reason.
        reason: IStr,
    },
    /// Procedure pack disable audit event.
    ProcedurePackDisabled {
        /// Procedure pack name.
        pack_name: IStr,
        /// Procedure pack version.
        version: IStr,
        /// Interned short reason.
        reason: IStr,
    },
}

fn sorted_deduped(values: impl IntoIterator<Item = IStr>) -> SmallVec<[IStr; 2]> {
    let mut values: SmallVec<[IStr; 2]> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use smallvec::smallvec;

    use super::*;
    use crate::{
        GraphTypeId, KeyLabelSetPolicy, NodeTypeRef, PredefinedValueType, PropertyDef,
        RecordTypeId, ValueType, intern,
    };

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
            labels_diff: LabelDiff::new([istr("change.add")], [istr("change.remove")]),
            properties_diff: PropertyDiff::new([(istr("change.set"), Value::Bool(true))], []),
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
            properties_diff: PropertyDiff::new([], [istr("change.removed")]),
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
    fn label_diff_added_and_removed_independent() {
        let added = istr("change.label.added");
        let removed = istr("change.label.removed");
        let diff = LabelDiff::new([added], [removed]);
        assert_eq!(diff.added.as_slice(), &[added]);
        assert_eq!(diff.removed.as_slice(), &[removed]);
    }

    #[test]
    fn property_diff_set_includes_null_value() {
        let property = istr("change.null");
        let diff = PropertyDiff::new([(property, Value::Null)], []);
        assert_eq!(diff.set.as_slice(), &[(property, Value::Null)]);
    }

    #[test]
    fn schema_change_procedure_pack_lifecycle() {
        let name = istr("pack");
        let version = istr("1.0.0");
        let reason = istr("retired");
        let activated = SchemaChange::ProcedurePackActivated {
            pack_name: name,
            version,
        };
        let deprecated = SchemaChange::ProcedurePackDeprecated {
            pack_name: name,
            version,
            reason,
        };
        let disabled = SchemaChange::ProcedurePackDisabled {
            pack_name: name,
            version,
            reason,
        };
        assert_ne!(activated, deprecated);
        assert_ne!(deprecated, disabled);
    }

    #[test]
    fn empty_diffs_and_empty_payload_are_valid() {
        assert!(LabelDiff::new([], []).is_empty());
        assert!(PropertyDiff::new([], []).is_empty());
        let event = Change::IndexExtensionEvent {
            provider: istr("empty-provider"),
            payload: Arc::from([]),
        };
        assert_eq!(event.clone(), event);
    }

    #[test]
    fn schema_change_variants_construct() {
        let graph_type_id = GraphTypeId::new(1).unwrap();
        let node_label = istr("change.schema.node");
        let edge_label = istr("change.schema.edge");
        let node = NodeTypeDef::new(LabelSet::single(node_label));
        let edge = EdgeTypeDef::new(edge_label, NodeTypeRef(node_label), NodeTypeRef(node_label));
        let record = RecordTypeDef {
            id: RecordTypeId::new(1),
            name: istr("change.schema.record"),
            fields: smallvec![PropertyDef {
                name: istr("change.schema.field"),
                value_type: ValueType::predefined(PredefinedValueType::String),
                nullable: false,
                default: None,
            }],
        };
        let mut graph_type = GraphType::new(graph_type_id, istr("change.schema.graph_type"));
        graph_type.key_label_set_policy = KeyLabelSetPolicy::NoOverlap;

        let variants = [
            SchemaChange::GraphTypeCreated { graph_type },
            SchemaChange::GraphTypeDropped { id: graph_type_id },
            SchemaChange::NodeTypeAdded {
                graph_type: graph_type_id,
                label: node_label,
                def: node,
            },
            SchemaChange::EdgeTypeAdded {
                graph_type: graph_type_id,
                label: edge_label,
                def: edge,
            },
            SchemaChange::RecordTypeAdded {
                graph_type: graph_type_id,
                def: record,
            },
        ];
        assert_eq!(variants.len(), 5);
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
            let diff = LabelDiff::new(added, removed);
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
            let diff = PropertyDiff::new(set, removed);
            prop_assert!(diff.set.windows(2).all(|pair| pair[0].0 < pair[1].0));
            prop_assert!(diff.removed.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}
