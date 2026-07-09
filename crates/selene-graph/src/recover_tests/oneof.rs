//! BRIEF-131e commit-3 recovery acceptance tests for
//! `EdgeEndpointDef::OneOf`.
//!
//! Each test follows the same shape as `variant_tests::recover_from_wal_*`:
//! drive a closed graph through `create_edge_type`, drop the snapshot, replay
//! the WAL via `recover_closed`, and assert structural identity. Sibling-
//! submodule per F2 so the parent `recover_tests.rs` stays under the 700 LOC
//! cap.

use std::fs;

use selene_core::{
    Change, EdgeEndpointDef as CoreEdgeEndpointDef, GraphId, GraphTypeId, LabelSet, NodeTypeRef,
    PropertyValueType, SchemaChange, db_string,
};
use smallvec::smallvec;

use crate::{
    EdgeEndpointDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, SharedGraph, ValidationMode,
};

use super::{append_wal, temp_dir};

fn three_node_type_graph() -> GraphTypeDef {
    let person = db_string("recover.oneof.person").unwrap();
    let company = db_string("recover.oneof.company").unwrap();
    let school = db_string("recover.oneof.school").unwrap();
    GraphTypeDef {
        name: db_string("recover.oneof.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(db_string("Person").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: company,
                key_labels: LabelSet::single(db_string("Company").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: school,
                key_labels: LabelSet::single(db_string("School").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    }
}

#[test]
fn wal_replay_oneof_edge_type() {
    // F5: drive an OneOf endpoint through create_edge_type + commit, omit the
    // snapshot, recover from WAL alone, and assert (i) indices resolved, (ii)
    // OneOf order preserved (sorted by `EdgeEndpointDef::one_of`), (iii)
    // length unchanged.
    let dir = temp_dir("wal-replay-oneof");
    let graph_id = GraphId::new(11310);
    let base = three_node_type_graph();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let rel = db_string("recover.oneof.affiliated_with").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_edge_type(
                rel.clone(),
                rel,
                EdgeEndpointDef::NodeType(0),
                EdgeEndpointDef::one_of([1, 2]),
                Vec::<PropertyTypeDef>::new(),
                ValidationMode::Strict,
            )
            .unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(graph_type.edge_types.len(), 1);
    let edge_type = &graph_type.edge_types[0];
    assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
    match &edge_type.target_node_type {
        EdgeEndpointDef::OneOf(indices) => {
            assert_eq!(indices.len(), 2);
            for window in indices.windows(2) {
                assert!(window[0] < window[1], "OneOf payload remains sorted");
            }
            assert_eq!(indices.as_slice(), &[1, 2]);
        }
        other => panic!("expected OneOf target endpoint, got {other:?}"),
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_replay_altered_oneof_edge_type() {
    let dir = temp_dir("wal-replay-oneof-altered");
    let graph_id = GraphId::new(11314);
    let base = three_node_type_graph();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let rel = db_string("recover.oneof.altered.affiliated_with").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_edge_type(
                rel.clone(),
                rel.clone(),
                EdgeEndpointDef::one_of([0, 1]),
                EdgeEndpointDef::NodeType(2),
                Vec::<PropertyTypeDef>::new(),
                ValidationMode::Strict,
            )
            .unwrap();
        mutator
            .alter_edge_type(
                rel,
                Some(EdgeEndpointDef::one_of([0, 1, 2])),
                None,
                vec![PropertyTypeDef {
                    name: db_string("commit_sha").unwrap(),
                    value_type: PropertyValueType::String,
                    list_element_type: None,
                    required: false,
                    default: None,
                    immutable: false,
                    unique: false,
                    decimal_type: None,
                    character_string_type: None,
                    byte_string_type: None,
                    record_field_types: None,
                }],
            )
            .unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(graph_type.edge_types.len(), 1);
    let edge_type = &graph_type.edge_types[0];
    assert_eq!(
        edge_type.source_node_type,
        EdgeEndpointDef::OneOf(vec![0, 1, 2])
    );
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(2));
    let property = edge_type
        .properties
        .iter()
        .find(|property| property.name == db_string("commit_sha").unwrap())
        .expect("altered optional property recovered");
    assert_eq!(property.value_type, PropertyValueType::String);
    assert!(!property.required);
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::SchemaChanged {
                change: SchemaChange::EdgeTypeAddedV2 { .. },
                ..
            },
            Change::SchemaChanged {
                change: SchemaChange::EdgeTypeAlteredV2 { .. },
                ..
            }
        ]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_replay_oneof_singleton_canonicalizes() {
    // Constructor invariant defense in depth: a WAL OneOf carrying a single
    // ref must collapse to NodeType on replay. We synthesize the WAL entry
    // directly (the mutator-side `EdgeEndpointDef::one_of` already collapses
    // singletons before emitting CoreEdgeEndpointDef, so a real-world WAL
    // would never reach this; this guards against a future regression where
    // a downstream emits a malformed OneOf and recovery has to canonicalize).
    let dir = temp_dir("wal-replay-oneof-singleton");
    let graph_id = GraphId::new(11311);
    let base = three_node_type_graph();
    let rel = db_string("recover.oneof.singleton").unwrap();
    let person = db_string("recover.oneof.person").unwrap();
    let company = db_string("recover.oneof.company").unwrap();
    append_wal(
        &dir,
        0,
        &[Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::EdgeTypeAddedV2 {
                graph_type: GraphTypeId::new(1).unwrap(),
                label: rel.clone(),
                def: selene_core::EdgeTypeDef {
                    label: rel,
                    source_node_type: CoreEdgeEndpointDef::NodeType(NodeTypeRef(person)),
                    target_node_type: CoreEdgeEndpointDef::OneOf(smallvec![NodeTypeRef(company)]),
                    properties: smallvec![],
                    validation_mode: selene_core::ValidationMode::Strict,
                },
            },
        }],
    );

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(graph_type.edge_types.len(), 1);
    let edge_type = &graph_type.edge_types[0];
    // Singleton OneOf must collapse to NodeType via the storage-side
    // `EdgeEndpointDef::one_of` constructor.
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(1));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_replay_edge_type_with_reordered_node_types_resolves_oneof_indices() {
    // F5: when the snapshotted GraphTypeDef has node types in a different
    // order from the original, OneOf indices must be re-resolved by name and
    // re-sorted. We construct a base whose node types are in REVERSE order
    // (School=0, Company=1, Person=2) and emit a WAL OneOf payload referring
    // by name; the resolver must look each up by NodeTypeRef and the storage
    // constructor must re-sort by the recovered index.
    let dir = temp_dir("wal-replay-oneof-reordered");
    let graph_id = GraphId::new(11312);
    let reversed = GraphTypeDef {
        name: db_string("recover.oneof.reversed.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: db_string("recover.oneof.school").unwrap(),
                key_labels: LabelSet::single(db_string("School").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("recover.oneof.company").unwrap(),
                key_labels: LabelSet::single(db_string("Company").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("recover.oneof.person").unwrap(),
                key_labels: LabelSet::single(db_string("Person").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    };
    let rel = db_string("recover.oneof.reordered.affiliated").unwrap();
    let person = db_string("recover.oneof.person").unwrap();
    let company = db_string("recover.oneof.company").unwrap();
    let school = db_string("recover.oneof.school").unwrap();
    append_wal(
        &dir,
        0,
        &[Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::EdgeTypeAddedV2 {
                graph_type: GraphTypeId::new(1).unwrap(),
                label: rel.clone(),
                def: selene_core::EdgeTypeDef {
                    label: rel,
                    source_node_type: CoreEdgeEndpointDef::NodeType(NodeTypeRef(person)),
                    // WAL refs in any order: (company, school) → must resolve
                    // to indices in the REVERSED graph and re-sort ascending.
                    target_node_type: CoreEdgeEndpointDef::OneOf(smallvec![
                        NodeTypeRef(company),
                        NodeTypeRef(school)
                    ]),
                    properties: smallvec![],
                    validation_mode: selene_core::ValidationMode::Strict,
                },
            },
        }],
    );

    let recovered = SharedGraph::recover_closed(&dir, graph_id, reversed).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    // In the reversed base: School=0, Company=1, Person=2. The WAL OneOf
    // [Company, School] resolves to indices [1, 0] then the storage
    // constructor sorts to [0, 1].
    let edge_type = &graph_type.edge_types[0];
    assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(2));
    assert_eq!(
        edge_type.target_node_type,
        EdgeEndpointDef::OneOf(vec![0, 1])
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn legacy_edge_type_def_v1_recovery_unchanged() {
    // Q8 grounding: EdgeTypeDefV1 -> EdgeTypeDef From-impl wraps in
    // EdgeEndpointDef::NodeType (never OneOf). Replay a legacy V1 WAL entry
    // and confirm the resulting endpoint is exactly NodeType.
    let dir = temp_dir("wal-replay-legacy-v1-unchanged");
    let graph_id = GraphId::new(11313);
    let base = three_node_type_graph();
    let rel = db_string("recover.oneof.legacy.knows").unwrap();
    let person = db_string("recover.oneof.person").unwrap();
    let company = db_string("recover.oneof.company").unwrap();
    append_wal(
        &dir,
        0,
        &[Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::EdgeTypeAdded {
                graph_type: GraphTypeId::new(1).unwrap(),
                label: rel.clone(),
                def: selene_core::EdgeTypeDefV1 {
                    label: rel,
                    source_node_type: NodeTypeRef(person),
                    target_node_type: NodeTypeRef(company),
                    properties: smallvec![],
                },
            },
        }],
    );

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    let edge_type = &graph_type.edge_types[0];
    // Legacy V1 paths produce NodeType only, never OneOf.
    assert!(
        !matches!(edge_type.source_node_type, EdgeEndpointDef::OneOf(_)),
        "legacy V1 must not produce OneOf"
    );
    assert!(
        !matches!(edge_type.target_node_type, EdgeEndpointDef::OneOf(_)),
        "legacy V1 must not produce OneOf"
    );
    assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(1));
    let _ = fs::remove_dir_all(dir);
}
