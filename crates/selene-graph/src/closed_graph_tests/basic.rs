use super::*;

#[test]
fn open_graph_commits_unchanged() {
    let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(db_string("Anything")), PropertyMap::new())
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
                LabelSet::single(db_string("Person")),
                prop("name", Value::String(db_string("Alice"))),
            )
            .unwrap()
    };
    txn.commit().unwrap();
    assert!(shared.read().is_node_alive(id));
}

#[test]
fn create_node_fills_declared_default_property() {
    let graph_type = GraphTypeDef {
        name: db_string("closed.default.graph"),
        node_types: vec![NodeTypeDef {
            name: db_string("closed.default.person"),
            key_labels: LabelSet::single(db_string("Person")),
            properties: vec![PropertyTypeDef {
                name: db_string("active"),
                value_type: PropertyValueType::Bool,
                list_element_type: None,
                required: false,
                default: Some(PropertyDefaultValue::Boolean(true)),
                immutable: false,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
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
        .create_node(LabelSet::single(db_string("Person")), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();

    assert_eq!(
        shared
            .read()
            .node_properties(id)
            .and_then(|properties| properties.get(&db_string("active"))),
        Some(&Value::Bool(true))
    );
}

#[test]
fn typed_list_property_rejects_wrong_element_type() {
    let graph_type = GraphTypeDef {
        name: db_string("closed.list.graph"),
        node_types: vec![NodeTypeDef {
            name: db_string("closed.list.person"),
            key_labels: LabelSet::single(db_string("Person")),
            properties: vec![PropertyTypeDef {
                name: db_string("tags"),
                value_type: PropertyValueType::List,
                list_element_type: Some(PropertyElementType::Scalar(PropertyValueType::String)),
                required: false,
                default: None,
                immutable: false,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
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
            LabelSet::single(db_string("Person")),
            prop(
                "tags",
                Value::List(vec![Value::String(db_string("ok")), Value::Int(7)]),
            ),
        )
        .unwrap();

    let err = txn.commit().unwrap_err();
    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::PropertyTypeMismatch { property, .. })
            if property == db_string("tags")
    ));
}

#[test]
fn immutable_property_update_is_rejected_before_commit() {
    let graph_type = GraphTypeDef {
        name: db_string("closed.immutable.graph"),
        node_types: vec![NodeTypeDef {
            name: db_string("closed.immutable.person"),
            key_labels: LabelSet::single(db_string("Person")),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: true,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
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
            LabelSet::single(db_string("Person")),
            prop("serial", Value::String(db_string("A"))),
        )
        .unwrap();
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .update_node(
            id,
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([(db_string("serial"), Value::String(db_string("B")))], []).unwrap(),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(id) && property == db_string("serial")
    ));
}

#[test]
fn warn_validation_mode_records_undeclared_property_warning() {
    let graph_type = GraphTypeDef {
        name: db_string("closed.warn.graph"),
        node_types: vec![NodeTypeDef {
            name: db_string("closed.warn.person"),
            key_labels: LabelSet::single(db_string("Person")),
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
            LabelSet::single(db_string("Person")),
            prop("extra", Value::Int(1)),
        )
        .unwrap();

    let outcome = txn.commit().unwrap();

    assert_eq!(outcome.warnings.len(), 1);
    assert!(matches!(
        &outcome.warnings[0].warning.violation,
        TypeViolation::UndeclaredProperty { property, .. } if *property == db_string("extra")
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
                .create_node(LabelSet::single(db_string("Person")), PropertyMap::new())
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
        }) if entity_id == EntityId::Node(NodeId::new(1)) && property == db_string("name")
    ));
    assert_eq!(shared.read().node_count(), 0);

    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(db_string("Person")),
                prop("name", Value::String(db_string("Bob"))),
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
                LabelSet::single(db_string("Person")),
                prop("name", Value::String(db_string("Alice"))),
            )
            .unwrap();
        let project = mutator
            .create_node(
                LabelSet::single(db_string("Project")),
                prop("name", Value::String(db_string("Apollo"))),
            )
            .unwrap();
        mutator
            .create_edge(db_string("KNOWS"), alice, project, PropertyMap::new())
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
                LabelSet::single(db_string("Stranger")), // would fail validation if checked
                prop("name", Value::String(db_string("scratch"))),
            )
            .unwrap();
        mutator.delete_node(scratch).unwrap();
    }
    txn.commit().expect(
        "create-then-delete in one tx must succeed even when the create's labels would \
         have failed validation; the validator skips tombstoned rows",
    );
}
