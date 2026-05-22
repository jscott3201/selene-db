use selene_core::{
    Change, GraphId, LabelSet, PropertyMap, PropertyValueType, SchemaChange, intern,
};

use crate::{
    EdgeTypeDef, GraphError, GraphTypeDef, NodeTypeDef, PropertyTypeDef, SharedGraph,
    TypeViolation, ValidationMode,
};

fn closed_empty_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: intern("catalog.empty").unwrap(),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn person_type() -> GraphTypeDef {
    let person = intern("Person").unwrap();
    GraphTypeDef {
        name: intern("catalog.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person,
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn person_company_type() -> GraphTypeDef {
    let person = intern("Person").unwrap();
    let company = intern("Company").unwrap();
    let works_at = intern("WORKS_AT").unwrap();
    GraphTypeDef {
        name: intern("catalog.company.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: company,
                key_labels: LabelSet::single(company),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: works_at,
            label: works_at,
            source_node_type: 0,
            target_node_type: 1,
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

#[test]
fn create_node_type_updates_bound_type_and_emits_schema_change() {
    let shared = closed_empty_graph(10);
    let person = intern("Person").unwrap();
    let name = intern("name").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node_type(
                    person,
                    LabelSet::single(person),
                    vec![PropertyTypeDef {
                        name,
                        value_type: PropertyValueType::String,
                        list_element_type: None,
                        required: true,
                        default: None,
                        immutable: false,
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
    let knows = intern("KNOWS").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_edge_type(knows, knows, 0, 0, Vec::new(), ValidationMode::Strict)
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
        .drop_node_type(intern("Person").unwrap())
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("would require reindexing")
    ));
}

#[test]
fn drop_edge_type_removes_type_and_emits_schema_change() {
    let shared = SharedGraph::builder(GraphId::new(13))
        .bound_to(person_company_type())
        .unwrap()
        .build()
        .unwrap();
    let works_at = intern("WORKS_AT").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator().drop_edge_type(works_at).unwrap();
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
    let person = intern("Person").unwrap();
    let err = txn
        .mutator()
        .create_node_type(
            person,
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
fn schema_change_commit_validates_full_entity_state() {
    let shared = SharedGraph::builder(GraphId::new(15))
        .bound_to(person_type())
        .unwrap()
        .build()
        .unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(intern("Person").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let mut txn = shared.begin_write();
    txn.mutator()
        .drop_node_type(intern("Person").unwrap())
        .expect("catalog mutation itself succeeds");
    let err = txn.commit().unwrap_err();

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::UnknownNodeLabel { .. })
    ));
}
