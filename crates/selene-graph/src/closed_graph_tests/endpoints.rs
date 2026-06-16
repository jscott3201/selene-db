use super::*;

fn person_company_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("closed.pc.graph"),
        node_types: vec![
            NodeTypeDef {
                name: db_string("closed.person.pc"),
                key_labels: LabelSet::single(db_string("PCPerson")),
                properties: vec![],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("closed.company.pc"),
                key_labels: LabelSet::single(db_string("PCCompany")),
                properties: vec![],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![crate::EdgeTypeDef {
            name: db_string("closed.works_at"),
            label: db_string("WORKS_AT"),
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
            .create_node(LabelSet::single(db_string("PCPerson")), PropertyMap::new())
            .unwrap();
        let company = mutator
            .create_node(LabelSet::single(db_string("PCCompany")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(db_string("WORKS_AT"), company, person, PropertyMap::new())
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
            .create_node(LabelSet::single(db_string("PCPerson")), PropertyMap::new())
            .unwrap();
        let project = mutator
            .create_node(LabelSet::single(db_string("PCProject")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(db_string("WORKS_AT"), person, project, PropertyMap::new())
            .unwrap();
    }

    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::UnknownNodeLabel { labels, .. })
            if labels == LabelSet::single(db_string("PCProject"))
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
            .create_node(LabelSet::single(db_string("PCPerson")), PropertyMap::new())
            .unwrap();
        let acme = mutator
            .create_node(LabelSet::single(db_string("PCCompany")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(db_string("WORKS_AT"), alice, acme, PropertyMap::new())
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
                LabelDiff::new([db_string("PCCompany")], [db_string("PCPerson")]).unwrap(),
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
