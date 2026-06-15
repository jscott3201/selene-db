use super::*;

#[test]
fn vector_score_expanded_candidates_accepts_gql_query_roots() {
    let graph = graph(330_223);
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, outgoing_near, outgoing_far, _) = seed_expanded_candidate_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[3.2, 0.0])));

    let table = execute_rows(
        &mut session,
        "MATCH (root:VectorRoot) \
         WITH collect_list(root) AS roots \
         CALL selene.vector_score_expanded_candidates('embedding', $query, roots, 'SUPPORTS', 4) \
         YIELD node_id, distance \
         RETURN node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![outgoing_near, root_a, outgoing_far, root_b]
    );
}

#[test]
fn vector_score_expanded_candidates_scores_preserved_roots_and_outgoing_expansion() {
    let graph = graph(330_217);
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, outgoing_near, outgoing_far, _) = seed_expanded_candidate_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[3.2, 0.0])));
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![
            Value::NodeRef(root_b),
            Value::NodeRef(root_a),
            Value::NodeRef(root_a),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_expanded_candidates('embedding', $query, $roots, 'SUPPORTS', 4) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![outgoing_near, root_a, outgoing_far, root_b]
    );
}

#[test]
fn vector_score_expanded_candidates_scores_incoming_expansion() {
    let graph = graph(330_218);
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, _, _, incoming) = seed_expanded_candidate_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[1.1, 0.0])));
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![Value::NodeRef(root_a), Value::NodeRef(root_b)]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_expanded_candidates('embedding', $query, $roots, 'SUPPORTS', 3, 'incoming', 'squared_euclidean') \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![incoming, root_a, root_b]
    );
}

#[test]
fn vector_score_expanded_candidates_rejects_non_node_roots() {
    let graph = graph(330_219);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(db_string("roots"), Value::List(vec![Value::Int(1)]));

    let err = session
        .execute_source(
            "CALL selene.vector_score_expanded_candidates('embedding', $query, $roots, 'SUPPORTS', 2)",
            &registry,
        )
        .expect_err("non-node roots must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("roots[0] must be a NODE")
    ));
}

#[test]
fn vector_score_expanded_candidates_batch_scores_per_query_expanded_roots() {
    let graph = graph(330_220);
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, outgoing_near, _, incoming) = seed_expanded_candidate_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![
            Value::Vector(vector(&[3.2, 0.0])),
            Value::Vector(vector(&[1.1, 0.0])),
        ]),
    );
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![
            Value::List(vec![
                Value::NodeRef(root_b),
                Value::NodeRef(root_a),
                Value::NodeRef(root_a),
            ]),
            Value::List(vec![Value::NodeRef(root_a), Value::NodeRef(root_b)]),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_expanded_candidates_batch('embedding', $queries, $roots, 'SUPPORTS', 3, 'both', 'squared_euclidean') \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 0, 1, 1, 1]);
    assert_eq!(
        node_column(&table, "node_id"),
        vec![
            outgoing_near,
            root_a,
            incoming,
            incoming,
            root_a,
            outgoing_near
        ]
    );
}

#[test]
fn vector_score_expanded_candidates_batch_rejects_mismatched_query_and_root_sets() {
    let graph = graph(330_221);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );
    session.bind_parameter(db_string("roots"), Value::List(Vec::new()));

    let err = session
        .execute_source(
            "CALL selene.vector_score_expanded_candidates_batch('embedding', $queries, $roots, 'SUPPORTS', 2)",
            &registry,
        )
        .expect_err("mismatched batch arity must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("queries and roots must have the same length")
    ));
}

#[test]
fn vector_score_expanded_candidates_batch_rejects_non_nested_node_roots() {
    let graph = graph(330_222);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![Value::List(vec![Value::Int(1)])]),
    );

    let err = session
        .execute_source(
            "CALL selene.vector_score_expanded_candidates_batch('embedding', $queries, $roots, 'SUPPORTS', 2)",
            &registry,
        )
        .expect_err("non-node nested roots must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("roots[0][0] must be a NODE")
    ));
}
