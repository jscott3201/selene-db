use selene_core::{
    Change, GraphId, LabelSet, PropertyMap, PropertyValueType, SchemaChange, db_string,
};

use crate::{
    DropBehavior, EdgeEndpointDef, EdgeTypeDef, GraphError, GraphTypeDef, NodeTypeDef,
    PropertyTypeDef, SharedGraph, ValidationMode,
};

fn closed_empty_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("catalog.empty").unwrap(),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn person_type() -> GraphTypeDef {
    let person = db_string("Person").unwrap();
    GraphTypeDef {
        name: db_string("catalog.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person.clone(),
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn person_company_type() -> GraphTypeDef {
    let person = db_string("Person").unwrap();
    let company = db_string("Company").unwrap();
    let works_at = db_string("WORKS_AT").unwrap();
    GraphTypeDef {
        name: db_string("catalog.company.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: person.clone(),
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: company.clone(),
                key_labels: LabelSet::single(company),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: works_at.clone(),
            label: works_at,
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(1),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

#[test]
fn create_node_type_updates_bound_type_and_emits_schema_change() {
    let shared = closed_empty_graph(10);
    let person = db_string("Person").unwrap();
    let name = db_string("name").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node_type(
                    person.clone(),
                    LabelSet::single(person.clone()),
                    vec![PropertyTypeDef {
                        name,
                        value_type: PropertyValueType::String,
                        list_element_type: None,
                        required: true,
                        default: None,
                        immutable: false,
                        unique: false,
                        record_field_types: None,
                    }],
                    ValidationMode::Strict,
                )
                .unwrap();
            assert_eq!(
                mutator
                    .read()
                    .meta
                    .bound_type
                    .as_ref()
                    .unwrap()
                    .node_types
                    .len(),
                1
            );
        }
        txn.commit().unwrap()
    };

    let graph_type = shared.graph_type().unwrap();
    assert_eq!(graph_type.node_types[0].name, person);
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAddedV2 { label, .. },
            ..
        }] if *label == person
    ));
}

#[test]
fn create_edge_type_resolves_closed_type_and_emits_schema_change() {
    let shared = SharedGraph::builder(GraphId::new(11))
        .bound_to(person_type())
        .unwrap()
        .build()
        .unwrap();
    let knows = db_string("KNOWS").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_edge_type(
                knows.clone(),
                knows.clone(),
                EdgeEndpointDef::NodeType(0),
                EdgeEndpointDef::NodeType(0),
                Vec::new(),
                ValidationMode::Strict,
            )
            .unwrap();
        txn.commit().unwrap()
    };

    let graph_type = shared.graph_type().unwrap();
    assert_eq!(graph_type.edge_types[0].name, knows);
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::EdgeTypeAddedV2 { label, .. },
            ..
        }] if *label == knows
    ));
}

#[test]
fn drop_node_type_refuses_endpoint_reindexing() {
    let shared = SharedGraph::builder(GraphId::new(12))
        .bound_to(person_company_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .drop_node_type(db_string("Person").unwrap(), DropBehavior::Restrict)
        .unwrap_err();

    // Person is referenced directly by WORKS_AT's source endpoint, so the
    // dependency check rejects with the "still references it" message before the
    // broader reindexing guard runs.
    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("still references it")
    ));
}

#[test]
fn drop_edge_type_removes_type_and_emits_schema_change() {
    let shared = SharedGraph::builder(GraphId::new(13))
        .bound_to(person_company_type())
        .unwrap()
        .build()
        .unwrap();
    let works_at = db_string("WORKS_AT").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .drop_edge_type(works_at.clone(), DropBehavior::Restrict)
            .unwrap();
        txn.commit().unwrap()
    };

    assert!(shared.graph_type().unwrap().edge_types.is_empty());
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::EdgeTypeDropped { name, .. },
            ..
        }] if *name == works_at
    ));
}

#[test]
fn catalog_type_ddl_on_open_graph_is_rejected() {
    let shared = SharedGraph::new(GraphId::new(14));
    let mut txn = shared.begin_write();
    let person = db_string("Person").unwrap();
    let err = txn
        .mutator()
        .create_node_type(
            person.clone(),
            LabelSet::single(person),
            Vec::new(),
            ValidationMode::Strict,
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("open graph (GG01) does not support catalog type DDL")
    ));
}

#[test]
fn drop_node_type_restrict_rejects_early_with_surviving_instances() {
    // Seam-B fix (audit Item 3): RESTRICT default rejects at the drop op itself
    // — not late at commit with a mislabelled UnknownNodeLabel — and leaves both
    // the type and its instances intact (no partial state).
    let shared = SharedGraph::builder(GraphId::new(15))
        .bound_to(person_type())
        .unwrap()
        .build()
        .unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(db_string("Person").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .drop_node_type(db_string("Person").unwrap(), DropBehavior::Restrict)
        .expect_err("RESTRICT rejects the drop op itself");
    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("1 instance(s) still exist") && reason.contains("CASCADE")
    ));
    drop(txn);

    // Nothing dropped, nothing removed.
    assert_eq!(shared.graph_type().unwrap().node_types.len(), 1);
    assert_eq!(shared.read().node_count(), 1);
}

#[test]
fn drop_node_type_cascade_truncates_then_drops_in_one_txn() {
    let shared = SharedGraph::builder(GraphId::new(150))
        .bound_to(person_type())
        .unwrap()
        .build()
        .unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(db_string("Person").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let person = db_string("Person").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .drop_node_type(person.clone(), DropBehavior::Cascade)
            .expect("CASCADE drops a type with surviving instances");
        txn.commit().unwrap()
    };

    // Both the truncate change and the schema drop landed in ONE committed
    // changeset, in order.
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodesOfTypeTruncated { label },
            Change::SchemaChanged {
                change: SchemaChange::NodeTypeDropped { name, .. },
                ..
            },
        ] if *label == person && *name == person
    ));
    assert!(shared.graph_type().unwrap().node_types.is_empty());
    assert_eq!(shared.read().node_count(), 0);
}

#[test]
fn drop_edge_type_restrict_rejects_early_with_surviving_instances() {
    let shared = SharedGraph::builder(GraphId::new(151))
        .bound_to(person_self_knows_type())
        .unwrap()
        .build()
        .unwrap();
    let person = db_string("Person").unwrap();
    let knows = db_string("KNOWS").unwrap();
    {
        let mut txn = shared.begin_write();
        let a = txn
            .mutator()
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
            .unwrap();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_edge(knows.clone(), a, b, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }

    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .drop_edge_type(knows, DropBehavior::Restrict)
        .expect_err("RESTRICT rejects edge-type drop with surviving edges");
    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("1 instance(s) still exist") && reason.contains("CASCADE")
    ));
    drop(txn);
    assert_eq!(shared.graph_type().unwrap().edge_types.len(), 1);
    assert_eq!(shared.read().edge_count(), 1);
}

#[test]
fn drop_edge_type_cascade_truncates_then_drops_in_one_txn() {
    let shared = SharedGraph::builder(GraphId::new(152))
        .bound_to(person_self_knows_type())
        .unwrap()
        .build()
        .unwrap();
    let person = db_string("Person").unwrap();
    let knows = db_string("KNOWS").unwrap();
    {
        let mut txn = shared.begin_write();
        let a = txn
            .mutator()
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
            .unwrap();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_edge(knows.clone(), a, b, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }

    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .drop_edge_type(knows.clone(), DropBehavior::Cascade)
            .expect("CASCADE drops an edge type with surviving edges");
        txn.commit().unwrap()
    };

    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::EdgesOfTypeTruncated { label },
            Change::SchemaChanged {
                change: SchemaChange::EdgeTypeDropped { name, .. },
                ..
            },
        ] if *label == knows && *name == knows
    ));
    assert!(shared.graph_type().unwrap().edge_types.is_empty());
    assert_eq!(shared.read().edge_count(), 0);
    // Nodes (the KNOWS endpoints) survive — only edges of the type were removed.
    assert_eq!(shared.read().node_count(), 2);
}

fn person_self_knows_type() -> GraphTypeDef {
    let person = db_string("Person").unwrap();
    let knows = db_string("KNOWS").unwrap();
    GraphTypeDef {
        name: db_string("catalog.person.knows.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person.clone(),
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![EdgeTypeDef {
            name: knows.clone(),
            label: knows,
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn person_company_school_with_oneof_edge_type() -> GraphTypeDef {
    let person = db_string("Person").unwrap();
    let company = db_string("Company").unwrap();
    let school = db_string("School").unwrap();
    let affiliated_with = db_string("AFFILIATED_WITH").unwrap();
    GraphTypeDef {
        name: db_string("catalog.oneof.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: person.clone(),
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: company.clone(),
                key_labels: LabelSet::single(company),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: school.clone(),
                key_labels: LabelSet::single(school),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: affiliated_with.clone(),
            label: affiliated_with,
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::one_of([1, 2]),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

#[test]
fn drop_node_type_rejects_when_oneof_endpoint_references_dropped_type() {
    // F4 fold: endpoint_depends_on_shifted_node previously used
    // node_type_index().is_some_and(...) which returns None for OneOf and
    // would have silently let the drop succeed. Explicit OneOf arm rejects
    // when any contained index >= removed_index.
    let shared = SharedGraph::builder(GraphId::new(16))
        .bound_to(person_company_school_with_oneof_edge_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .drop_node_type(db_string("Company").unwrap(), DropBehavior::Restrict)
        .unwrap_err();

    // Company (index 1) is directly carried by OneOf([1, 2]); the dependency
    // check rejects with the clear "still references it" message.
    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("still references it")
    ));
}

#[test]
fn drop_node_type_rejects_when_oneof_endpoint_references_tail_type() {
    // Tail-index drop (School at index 2) is rejected because OneOf([1, 2])
    // carries School directly; the dependency check rejects it as a dangling
    // endpoint rather than rewriting OneOf payloads in place.
    let shared = SharedGraph::builder(GraphId::new(17))
        .bound_to(person_company_school_with_oneof_edge_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .drop_node_type(db_string("School").unwrap(), DropBehavior::Restrict)
        .unwrap_err();

    // School (index 2) is also directly carried by OneOf([1, 2]); the dependency
    // check rejects with the clear "still references it" message.
    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("still references it")
    ));
}
