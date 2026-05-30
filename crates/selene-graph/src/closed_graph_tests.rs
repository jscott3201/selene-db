use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{
    Change, EdgeId, GraphId, HlcTimestamp, LabelDiff, LabelSet, NodeId, Origin, PropertyDiff,
    PropertyMap, PropertyValueType, Value, intern,
};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotBuilder, SnapshotConfig, SyncPolicy,
    WalConfig, WalWriter,
};

use crate::{
    CORE_PROVIDER_TAG, EdgeEndpointDef, EntityId, GraphError, GraphTypeDef, NodeTypeDef,
    PropertyDefaultValue, PropertyElementType, PropertyTypeDef, ProviderTag, SharedGraph,
    TypeViolation, ValidationMode,
};

#[path = "closed_graph_tests/immutable.rs"]
mod immutable;

#[path = "closed_graph_tests/one_of.rs"]
mod one_of;

#[path = "closed_graph_tests/truncate.rs"]
mod truncate;

fn istr(name: &str) -> selene_core::IStr {
    intern(name).unwrap()
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(istr(name), value)]).unwrap()
}

fn person_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("closed.person.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("closed.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![PropertyTypeDef {
                name: istr("name"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: false,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![crate::EdgeTypeDef {
            name: istr("closed.knows"),
            label: istr("KNOWS"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: vec![PropertyTypeDef {
                name: istr("since"),
                value_type: PropertyValueType::Int,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-closed-graph-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn write_snapshot(dir: &Path, shared: &SharedGraph, sequence: u64) {
    let provider = shared
        .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
        .expect("core provider is registered");
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.to_path_buf(),
        sequence,
        compression: SectionCompression::None,
        fsync: false,
    });
    for sub in provider.declared_sub_tags() {
        let bytes = provider.write_section(*sub).unwrap();
        builder
            .add_section(CORE_PROVIDER_TAG, sub.0, bytes)
            .unwrap();
    }
    builder.finalize().unwrap();
}

fn append_wal(dir: &Path, snapshot_seq: u64, changes: &[Change]) {
    let mut writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq,
        },
    )
    .unwrap();
    writer
        .append(HlcTimestamp::zero(), Origin::Local, None, changes)
        .unwrap();
    writer.flush().unwrap();
}

#[test]
fn open_graph_commits_unchanged() {
    let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(istr("Anything")), PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(!shared.is_closed());
    assert_eq!(shared.read().node_count(), 1);
}

#[test]
fn closed_graph_accepts_valid_commit() {
    let shared = SharedGraph::builder(GraphId::new(2))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap()
    };
    txn.commit().unwrap();
    assert!(shared.read().is_node_alive(id));
}

#[test]
fn create_node_fills_declared_default_property() {
    let graph_type = GraphTypeDef {
        name: istr("closed.default.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("closed.default.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![PropertyTypeDef {
                name: istr("active"),
                value_type: PropertyValueType::Bool,
                list_element_type: None,
                required: false,
                default: Some(PropertyDefaultValue::Boolean(true)),
                immutable: false,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(20))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();

    assert_eq!(
        shared
            .read()
            .node_properties(id)
            .and_then(|properties| properties.get(&istr("active"))),
        Some(&Value::Bool(true))
    );
}

#[test]
fn typed_list_property_rejects_wrong_element_type() {
    let graph_type = GraphTypeDef {
        name: istr("closed.list.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("closed.list.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![PropertyTypeDef {
                name: istr("tags"),
                value_type: PropertyValueType::List,
                list_element_type: Some(PropertyElementType::Scalar(PropertyValueType::String)),
                required: false,
                default: None,
                immutable: false,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(22))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(istr("Person")),
            prop(
                "tags",
                Value::List(vec![Value::String(istr("ok")), Value::Int(7)]),
            ),
        )
        .unwrap();

    let err = txn.commit().unwrap_err();
    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::PropertyTypeMismatch { property, .. })
            if property == istr("tags")
    ));
}

#[test]
fn immutable_property_update_is_rejected_before_commit() {
    let graph_type = GraphTypeDef {
        name: istr("closed.immutable.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("closed.immutable.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![PropertyTypeDef {
                name: istr("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: true,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(21))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(
            LabelSet::single(istr("Person")),
            prop("serial", Value::String(istr("A"))),
        )
        .unwrap();
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .update_node(
            id,
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([(istr("serial"), Value::String(istr("B")))], []).unwrap(),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(id) && property == istr("serial")
    ));
}

#[test]
fn warn_validation_mode_records_undeclared_property_warning() {
    let graph_type = GraphTypeDef {
        name: istr("closed.warn.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("closed.warn.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: Vec::new(),
            validation_mode: ValidationMode::Warn,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(22))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(istr("Person")),
            prop("extra", Value::Int(1)),
        )
        .unwrap();

    let outcome = txn.commit().unwrap();

    assert_eq!(outcome.warnings.len(), 1);
    assert!(matches!(
        outcome.warnings[0].warning.violation,
        TypeViolation::UndeclaredProperty { property, .. } if property == istr("extra")
    ));
}

#[test]
fn closed_graph_rejects_invalid_commit_without_publishing() {
    let shared = SharedGraph::builder(GraphId::new(3))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        assert_eq!(
            mutator
                .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
                .unwrap(),
            NodeId::new(1)
        );
    }
    let err = txn.commit().unwrap_err();
    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::MissingRequiredProperty {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(NodeId::new(1)) && property == istr("name")
    ));
    assert_eq!(shared.read().node_count(), 0);

    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Bob"))),
            )
            .unwrap()
    };
    assert_eq!(id, NodeId::new(2), "D11 allocator hole is preserved");
    txn.commit().unwrap();
}

#[test]
fn closed_graph_rejects_edge_endpoint_mismatch() {
    let shared = SharedGraph::builder(GraphId::new(4))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let alice = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap();
        let project = mutator
            .create_node(
                LabelSet::single(istr("Project")),
                prop("name", Value::String(istr("Apollo"))),
            )
            .unwrap();
        mutator
            .create_edge(istr("KNOWS"), alice, project, PropertyMap::new())
            .unwrap();
    }
    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::UnknownNodeLabel {
            id,
            ..
        }) if id == NodeId::new(2)
    ));
}

#[test]
fn recover_round_trips_bound_graph_type_and_rearms_validator() {
    let dir = temp_dir("roundtrip");
    let graph_type = person_graph_type();
    let shared = SharedGraph::builder(GraphId::new(5))
        .bound_to(graph_type.clone())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let alice = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap();
        let bob = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Bob"))),
            )
            .unwrap();
        mutator
            .create_edge(istr("KNOWS"), alice, bob, prop("since", Value::Int(2026)))
            .unwrap();
    }
    txn.commit().unwrap();
    let sequence = shared.read().meta.generation;
    write_snapshot(&dir, &shared, sequence);
    append_wal(
        &dir,
        sequence,
        &[Change::NodeCreated {
            id: NodeId::new(3),
            labels: LabelSet::single(istr("Person")),
            properties: prop("name", Value::String(istr("Carol"))),
        }],
    );

    let recovered = SharedGraph::recover_closed(&dir, GraphId::new(5), graph_type.clone()).unwrap();
    assert!(recovered.is_closed());
    assert_eq!(recovered.graph_type().as_deref(), Some(&graph_type));
    assert!(recovered.read().is_node_alive(NodeId::new(3)));

    let mut txn = recovered.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_edge(
                istr("KNOWS"),
                NodeId::new(1),
                NodeId::new(2),
                prop("since", Value::String(istr("bad"))),
            )
            .unwrap();
    }
    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::PropertyTypeMismatch {
            entity_id,
            property,
            expected: PropertyValueType::Int,
            observed: "String",
        }) if entity_id == EntityId::Edge(EdgeId::new(2)) && property == istr("since")
    ));
    let _ = fs::remove_dir_all(dir);
}

fn person_company_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("closed.pc.graph"),
        node_types: vec![
            NodeTypeDef {
                name: istr("closed.person.pc"),
                key_labels: LabelSet::single(istr("PCPerson")),
                properties: vec![],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("closed.company.pc"),
                key_labels: LabelSet::single(istr("PCCompany")),
                properties: vec![],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![crate::EdgeTypeDef {
            name: istr("closed.works_at"),
            label: istr("WORKS_AT"),
            source_node_type: EdgeEndpointDef::NodeType(0), // PCPerson
            target_node_type: EdgeEndpointDef::NodeType(1), // PCCompany
            properties: vec![],
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn any_edge_person_company_graph_type() -> GraphTypeDef {
    let mut graph_type = person_company_graph_type();
    graph_type.edge_types[0].source_node_type = EdgeEndpointDef::Any;
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::Any;
    graph_type
}

#[test]
fn closed_graph_any_edge_accepts_declared_endpoint_types() {
    let shared = SharedGraph::builder(GraphId::new(25))
        .bound_to(any_edge_person_company_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let person = mutator
            .create_node(LabelSet::single(istr("PCPerson")), PropertyMap::new())
            .unwrap();
        let company = mutator
            .create_node(LabelSet::single(istr("PCCompany")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(istr("WORKS_AT"), company, person, PropertyMap::new())
            .unwrap();
    }

    txn.commit()
        .expect("Any endpoints accept all declared node types");
}

#[test]
fn closed_graph_any_edge_rejects_undeclared_endpoint_type() {
    let shared = SharedGraph::builder(GraphId::new(26))
        .bound_to(any_edge_person_company_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let person = mutator
            .create_node(LabelSet::single(istr("PCPerson")), PropertyMap::new())
            .unwrap();
        let project = mutator
            .create_node(LabelSet::single(istr("PCProject")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(istr("WORKS_AT"), person, project, PropertyMap::new())
            .unwrap();
    }

    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::UnknownNodeLabel { labels, .. })
            if labels == LabelSet::single(istr("PCProject"))
    ));
}

#[test]
fn closed_graph_revalidates_incident_edges_on_node_label_change() {
    // F1 regression: a NodeUpdated that flips labels can leave incident edges
    // pointing to mismatched endpoint types without producing an EdgeUpdated.
    // The validator must fan out to incident edges.
    let graph_type = person_company_graph_type();
    let shared = SharedGraph::builder(GraphId::new(11))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let (alice, acme) = {
        let mut mutator = txn.mutator();
        let alice = mutator
            .create_node(LabelSet::single(istr("PCPerson")), PropertyMap::new())
            .unwrap();
        let acme = mutator
            .create_node(LabelSet::single(istr("PCCompany")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(istr("WORKS_AT"), alice, acme, PropertyMap::new())
            .unwrap();
        (alice, acme)
    };
    txn.commit().unwrap();

    // Flip Alice from PCPerson to PCCompany. The (label=WORKS_AT,
    // source_type=PCPerson, target_type=PCCompany) edge no longer matches
    // any edge type because the source side is now PCCompany.
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .update_node(
                alice,
                LabelDiff::new([istr("PCCompany")], [istr("PCPerson")]).unwrap(),
                PropertyDiff::new(std::iter::empty(), std::iter::empty()).unwrap(),
            )
            .unwrap();
    }
    let err = txn.commit().unwrap_err();
    assert!(
        matches!(
            err,
            GraphError::TypeViolation(TypeViolation::EdgeEndpointTypeMismatch { .. })
        ),
        "expected EdgeEndpointTypeMismatch, got {err:?}",
    );
    // Original endpoints still alive — no publication happened.
    assert!(shared.read().is_node_alive(alice));
    assert!(shared.read().is_node_alive(acme));
}

#[test]
fn closed_graph_accepts_create_then_delete_in_same_tx() {
    // F6 regression: creating then deleting an entity in one commit should
    // succeed; the validator must skip entities not alive in the working
    // snapshot rather than failing on UnknownNodeLabel for a tombstoned row.
    let graph_type = person_graph_type();
    let shared = SharedGraph::builder(GraphId::new(12))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let scratch = mutator
            .create_node(
                LabelSet::single(istr("Stranger")), // would fail validation if checked
                prop("name", Value::String(istr("scratch"))),
            )
            .unwrap();
        mutator.delete_node(scratch).unwrap();
    }
    txn.commit().expect(
        "create-then-delete in one tx must succeed even when the create's labels would \
         have failed validation; the validator skips tombstoned rows",
    );
}

#[test]
fn from_graph_validates_bound_type_self_consistency() {
    // F4 regression: SharedGraph::from_graph must validate the bound_type
    // shape, otherwise a malformed type slipped through the public
    // GraphMeta surface accepts a graph that builder().bound_to() would
    // have rejected.
    use crate::SeleneGraph;
    let mut bad_type = person_company_graph_type();
    bad_type.edge_types[0].source_node_type = EdgeEndpointDef::NodeType(99); // out of range
    let mut graph = SeleneGraph::new(GraphId::new(13));
    graph.meta.bound_type = Some(std::sync::Arc::new(bad_type));
    let result = SharedGraph::try_from_graph(graph);
    assert!(matches!(
        result,
        Err(GraphError::Inconsistent { reason })
            if reason.contains("references node type index")
    ));
}

#[test]
fn recover_closed_preserves_bound_type_for_wal_only() {
    // F2 regression: WAL-only recovery (no snapshot) must accept the
    // caller's bound_type rather than silently defaulting to None.
    // Without this, a closed-graph crash before the first snapshot would
    // permanently downgrade to open and skip GG02 validation forever after.
    let dir = temp_dir("closed-wal-only");
    let graph_type = person_graph_type();
    append_wal(
        &dir,
        0,
        &[Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(istr("Person")),
            properties: prop("name", Value::String(istr("Alice"))),
        }],
    );

    let recovered =
        SharedGraph::recover_closed(&dir, GraphId::new(14), graph_type.clone()).unwrap();
    assert!(recovered.is_closed());
    assert_eq!(recovered.graph_type().as_deref(), Some(&graph_type));
    assert!(recovered.read().is_node_alive(NodeId::new(1)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_closed_rejects_disagreement_with_snapshot_meta() {
    // F2 regression (drift case): if the snapshot's META declares one
    // bound_type and the caller asserts a different one, recovery must fail
    // rather than silently picking either side.
    let dir = temp_dir("closed-drift");
    let snapshot_type = person_graph_type();
    let shared = SharedGraph::builder(GraphId::new(15))
        .bound_to(snapshot_type)
        .unwrap()
        .build()
        .unwrap();
    write_snapshot(&dir, &shared, 1);

    let mut other_type = person_graph_type();
    other_type.name = istr("closed.person.other");
    let err = match SharedGraph::recover_closed(&dir, GraphId::new(15), other_type) {
        Ok(_) => panic!("recovery should fail on bound_type drift"),
        Err(error) => error,
    };
    let GraphError::Provider(crate::ProviderError::Inconsistent { reason }) = &err else {
        panic!("expected Provider::Inconsistent, got {err:?}");
    };
    assert!(
        reason.contains("bound_type disagrees"),
        "expected bound_type-disagrees, got: {reason}",
    );
    let _ = fs::remove_dir_all(dir);
}
