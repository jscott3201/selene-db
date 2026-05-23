use super::*;

fn mentions_one_of_graph_type() -> GraphTypeDef {
    // OneOf fixture per §A.4: a MENTIONS edge whose source enumerates
    // {Document, Comment} and target is the single Topic node type.
    GraphTypeDef {
        name: istr("closed.mentions_one_of.graph"),
        node_types: vec![
            NodeTypeDef {
                name: istr("closed.document"),
                key_labels: LabelSet::single(istr("Document")),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("closed.comment"),
                key_labels: LabelSet::single(istr("Comment")),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("closed.topic"),
                key_labels: LabelSet::single(istr("Topic")),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![crate::EdgeTypeDef {
            name: istr("closed.mentions"),
            label: istr("MENTIONS"),
            source_node_type: EdgeEndpointDef::one_of([0, 1]),
            target_node_type: EdgeEndpointDef::NodeType(2),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

#[test]
fn closed_graph_one_of_endpoint_accepts_either_declared_member() {
    // OneOf fixture coverage per §A.4: source enumerates {Document, Comment}
    // → topic, so MENTIONS edges from EITHER source kind must validate. The
    // matches_node_type membership lookup on OneOf is what makes both pass.
    let shared = SharedGraph::builder(GraphId::new(110))
        .bound_to(mentions_one_of_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let topic = mutator
            .create_node(LabelSet::single(istr("Topic")), PropertyMap::new())
            .unwrap();
        let document = mutator
            .create_node(LabelSet::single(istr("Document")), PropertyMap::new())
            .unwrap();
        let comment = mutator
            .create_node(LabelSet::single(istr("Comment")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(istr("MENTIONS"), document, topic, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(istr("MENTIONS"), comment, topic, PropertyMap::new())
            .unwrap();
    }
    txn.commit()
        .expect("OneOf endpoint accepts every declared candidate node type");
}

#[test]
fn closed_graph_one_of_endpoint_rejects_non_member_source() {
    // OneOf negative case: Topic is not in source set {Document, Comment}.
    let shared = SharedGraph::builder(GraphId::new(111))
        .bound_to(mentions_one_of_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(LabelSet::single(istr("Topic")), PropertyMap::new())
            .unwrap();
        let b = mutator
            .create_node(LabelSet::single(istr("Topic")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(istr("MENTIONS"), a, b, PropertyMap::new())
            .unwrap();
    }
    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::EdgeEndpointTypeMismatch { .. })
    ));
}
