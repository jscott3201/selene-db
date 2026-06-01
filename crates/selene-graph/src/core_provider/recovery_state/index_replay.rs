//! Post-snapshot WAL index-intent accumulation and registration-set replay.
//!
//! Split out of `recovery_state.rs` (700-LOC cap). `RecoveryState::apply_change`
//! distills each property/composite-index `SchemaChange` into a [`PendingIndex`]
//! / [`PendingCompositeIndex`] intent; `into_graph` then replays those intents
//! against the **registration set only** (empty `TypedIndex` placeholders). The
//! single bitmap rebuild runs downstream in
//! `SharedGraph::from_graph_parts_and_snapshot` (GRAPH-06 dedup).

use selene_core::{IStr, SchemaChange, SchemaPropertyIndexKind};
use smallvec::SmallVec;

use crate::graph::{CompositePropertyIndexEntry, PropertyIndexEntry, SeleneGraph};
use crate::typed_index::TypedIndex;
use crate::typed_index::TypedIndexKind;

/// A distilled, replayable property-index intent from the WAL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PendingIndex {
    /// Register an index for `(label, property)` of the declared `kind`.
    Create {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
        /// Declared indexable value kind.
        kind: TypedIndexKind,
        /// Optional explicit catalog name.
        name: Option<IStr>,
    },
    /// Drop the index registration for `(label, property)`.
    Drop {
        /// Indexed node label.
        label: IStr,
        /// Indexed property key.
        property: IStr,
    },
}

/// A distilled, replayable composite-property-index intent from the WAL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PendingCompositeIndex {
    /// Register a composite index over `(label, properties...)`.
    Create {
        /// Indexed node label.
        label: IStr,
        /// Indexed property keys in declaration order.
        properties: SmallVec<[IStr; 4]>,
        /// Declared indexable value kinds in declaration order.
        kinds: SmallVec<[TypedIndexKind; 4]>,
        /// Optional explicit catalog name.
        name: Option<IStr>,
    },
    /// Drop the composite index registration over `(label, properties...)`.
    Drop {
        /// Indexed node label.
        label: IStr,
        /// Indexed property keys in declaration order.
        properties: SmallVec<[IStr; 4]>,
    },
}

/// Distill a property-index `SchemaChange` into a replayable intent, or `None`
/// when the change is not a property-index change.
pub(super) fn pending_property_index_change(change: &SchemaChange) -> Option<PendingIndex> {
    match change {
        SchemaChange::PropertyIndexCreated {
            label,
            property,
            kind,
        } => Some(PendingIndex::Create {
            label: label.clone(),
            property: property.clone(),
            kind: typed_kind_from(*kind),
            name: None,
        }),
        SchemaChange::PropertyIndexCreatedNamed {
            label,
            property,
            kind,
            name,
        } => Some(PendingIndex::Create {
            label: label.clone(),
            property: property.clone(),
            kind: typed_kind_from(*kind),
            name: name.clone(),
        }),
        SchemaChange::PropertyIndexDropped { label, property } => Some(PendingIndex::Drop {
            label: label.clone(),
            property: property.clone(),
        }),
        _ => None,
    }
}

/// Replay post-snapshot WAL property-index intents against the **registration
/// set only** — create inserts an empty `TypedIndex` placeholder of the declared
/// kind, drop removes the registration. The bitmap contents are (re)built once,
/// downstream, by `SharedGraph::from_graph_parts_and_snapshot`'s single
/// `rebuild_property_indexes` pass (which reads this registration set), so
/// `into_graph` no longer builds index contents that the downstream rebuild
/// immediately discards (GRAPH-06).
pub(super) fn replay_property_index_changes(
    graph: &mut SeleneGraph,
    changes: &[PendingIndex],
) -> crate::GraphResult<()> {
    for change in changes {
        match change {
            PendingIndex::Create {
                label,
                property,
                kind,
                name,
            } => {
                graph.property_index.insert(
                    (label.clone(), property.clone()),
                    PropertyIndexEntry::new(TypedIndex::new(*kind), name.clone()),
                );
            }
            PendingIndex::Drop { label, property } => {
                graph
                    .property_index
                    .remove(&(label.clone(), property.clone()));
            }
        }
    }
    Ok(())
}

/// Distill a composite-property-index `SchemaChange` into a replayable intent,
/// or `None` when the change is not a composite-index change.
pub(super) fn pending_composite_property_index_change(
    change: &SchemaChange,
) -> Option<PendingCompositeIndex> {
    match change {
        SchemaChange::CompositePropertyIndexCreated {
            label,
            properties,
            kinds,
            name,
        } => Some(PendingCompositeIndex::Create {
            label: label.clone(),
            properties: properties.clone(),
            kinds: kinds.iter().copied().map(typed_kind_from).collect(),
            name: name.clone(),
        }),
        SchemaChange::CompositePropertyIndexDropped { label, properties } => {
            Some(PendingCompositeIndex::Drop {
                label: label.clone(),
                properties: properties.clone(),
            })
        }
        _ => None,
    }
}

/// Replay post-snapshot WAL composite-index intents against the **registration
/// set only** (see [`replay_property_index_changes`]); the downstream
/// `rebuild_composite_property_indexes` pass fills the bitmaps once.
pub(super) fn replay_composite_property_index_changes(
    graph: &mut SeleneGraph,
    changes: &[PendingCompositeIndex],
) -> crate::GraphResult<()> {
    for change in changes {
        match change {
            PendingCompositeIndex::Create {
                label,
                properties,
                kinds,
                name,
            } => {
                let key = crate::graph::composite_property_key(properties);
                graph.composite_property_index.insert(
                    (label.clone(), key),
                    CompositePropertyIndexEntry::new(
                        crate::CompositeTypedIndex::new(kinds.clone()),
                        properties.clone(),
                        name.clone(),
                    ),
                );
            }
            PendingCompositeIndex::Drop { label, properties } => {
                let key = crate::graph::composite_property_key(properties);
                graph.composite_property_index.remove(&(label.clone(), key));
            }
        }
    }
    Ok(())
}

/// Map a persisted `SchemaPropertyIndexKind` to the in-memory `TypedIndexKind`.
pub(super) const fn typed_kind_from(kind: SchemaPropertyIndexKind) -> TypedIndexKind {
    match kind {
        SchemaPropertyIndexKind::I64 => TypedIndexKind::I64,
        SchemaPropertyIndexKind::F64 => TypedIndexKind::F64,
        SchemaPropertyIndexKind::String => TypedIndexKind::String,
        SchemaPropertyIndexKind::Date => TypedIndexKind::Date,
        SchemaPropertyIndexKind::LocalDateTime => TypedIndexKind::LocalDateTime,
        SchemaPropertyIndexKind::Uuid => TypedIndexKind::Uuid,
    }
}
