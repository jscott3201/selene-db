use super::*;

#[test]
fn vector_score_neighbors_reranks_directional_graph_candidates() {
    let graph = graph(330_214);
    let registry = BuiltinProcedureRegistry::new();
    let (anchor, _, ids) = seed_neighbor_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[2.2, 0.0])));
    session.bind_parameter(db_string("anchor"), Value::NodeRef(anchor));

    let outgoing = execute_rows(
        &mut session,
        "CALL selene.vector_score_neighbors('embedding', $query, $anchor, 'DEPENDS_ON', 3, 'outgoing', 'squared_euclidean') \
         YIELD node_id, distance",
        &registry,
    );
    assert_eq!(
        node_column(&outgoing, "node_id"),
        vec![ids[2], ids[0], ids[5]]
    );

    let incoming = execute_rows(
        &mut session,
        "CALL selene.vector_score_neighbors('embedding', $query, $anchor, 'DEPENDS_ON', 3, 'incoming', 'squared_euclidean') \
         YIELD node_id, distance",
        &registry,
    );
    assert_eq!(node_column(&incoming, "node_id"), vec![ids[1]]);
}

#[test]
fn vector_score_neighbors_batch_reranks_per_anchor_neighbor_sets() {
    let graph = graph(330_215);
    let registry = BuiltinProcedureRegistry::new();
    let (anchor, second_anchor, ids) = seed_neighbor_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![
            Value::Vector(vector(&[2.2, 0.0])),
            Value::Vector(vector(&[6.2, 0.0])),
        ]),
    );
    session.bind_parameter(
        db_string("anchors"),
        Value::List(vec![Value::NodeRef(anchor), Value::NodeRef(second_anchor)]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_neighbors_batch('embedding', $queries, $anchors, 'DEPENDS_ON', 2, 'outgoing', 'squared_euclidean') \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    assert_eq!(
        node_column(&table, "node_id"),
        vec![ids[2], ids[0], ids[6], ids[7]]
    );
}

#[test]
fn vector_score_neighbors_rejects_invalid_direction() {
    let graph = graph(330_216);
    let registry = BuiltinProcedureRegistry::new();
    let (anchor, _, _) = seed_neighbor_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(db_string("anchor"), Value::NodeRef(anchor));

    let err = session
        .execute_source(
            "CALL selene.vector_score_neighbors('embedding', $query, $anchor, 'DEPENDS_ON', 2, 'sideways')",
            &registry,
        )
        .expect_err("invalid direction must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("unknown vector neighbor direction")
    ));
}
