//! End-to-end coverage for native vector candidate-scoring built-ins.

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(value)) => *value,
            other => panic!("expected node ref in {name}, got {other:?}"),
        })
        .collect()
}

fn uint_column(table: &BindingTable, name: &str) -> Vec<u64> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Uint(value)) => *value,
            other => panic!("expected uint in {name}, got {other:?}"),
        })
        .collect()
}

fn seed_vector_graph(graph: &SharedGraph) -> Vec<NodeId> {
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    let other = istr("other");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let mut ids = Vec::new();
    for i in 0..8 {
        ids.push(
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts"),
        );
    }
    ids.push(
        mutator
            .create_node(
                LabelSet::single(doc),
                props(&other, Value::String(istr("not-a-vector"))),
            )
            .expect("non-vector node inserts"),
    );
    txn.commit().expect("seed graph commits");
    ids
}

fn seed_neighbor_vector_graph(graph: &SharedGraph) -> (NodeId, NodeId, Vec<NodeId>) {
    let anchor_label = istr("Anchor");
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    let other = istr("other");
    let depends = istr("DEPENDS_ON");
    let mentions = istr("MENTIONS");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let anchor = mutator
        .create_node(LabelSet::single(anchor_label.clone()), PropertyMap::new())
        .expect("anchor inserts");
    let second_anchor = mutator
        .create_node(LabelSet::single(anchor_label), PropertyMap::new())
        .expect("second anchor inserts");
    let mut ids = Vec::new();
    for i in 0..8 {
        ids.push(
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts"),
        );
    }
    let non_vector = mutator
        .create_node(
            LabelSet::single(doc),
            props(&other, Value::String(istr("not-a-vector"))),
        )
        .expect("non-vector node inserts");
    for &node in &[ids[5], ids[2], ids[2], ids[0], non_vector] {
        mutator
            .create_edge(depends.clone(), anchor, node, PropertyMap::new())
            .expect("outgoing dependency edge inserts");
    }
    mutator
        .create_edge(mentions, anchor, ids[7], PropertyMap::new())
        .expect("other edge inserts");
    mutator
        .create_edge(depends.clone(), ids[1], anchor, PropertyMap::new())
        .expect("incoming dependency edge inserts");
    for &node in &[ids[4], ids[6], ids[7]] {
        mutator
            .create_edge(depends.clone(), second_anchor, node, PropertyMap::new())
            .expect("second anchor edge inserts");
    }
    txn.commit().expect("seed graph commits");
    (anchor, second_anchor, ids)
}

fn seed_expanded_candidate_graph(graph: &SharedGraph) -> (NodeId, NodeId, NodeId, NodeId, NodeId) {
    let doc = istr("VectorDoc");
    let root = istr("VectorRoot");
    let embedding = istr("embedding");
    let other = istr("other");
    let support = istr("SUPPORTS");
    let mentions = istr("MENTIONS");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let root_labels = || LabelSet::from_iter([doc.clone(), root.clone()]);
    let root_a = mutator
        .create_node(
            root_labels(),
            props(&embedding, Value::Vector(vector(&[2.0, 0.0]))),
        )
        .expect("root_a inserts");
    let root_b = mutator
        .create_node(
            root_labels(),
            props(&embedding, Value::Vector(vector(&[8.0, 0.0]))),
        )
        .expect("root_b inserts");
    let outgoing_near = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&embedding, Value::Vector(vector(&[3.0, 0.0]))),
        )
        .expect("outgoing near inserts");
    let outgoing_far = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&embedding, Value::Vector(vector(&[7.0, 0.0]))),
        )
        .expect("outgoing far inserts");
    let incoming = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
        )
        .expect("incoming inserts");
    let wrong_label = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&embedding, Value::Vector(vector(&[4.0, 0.0]))),
        )
        .expect("wrong-label node inserts");
    let non_vector = mutator
        .create_node(
            LabelSet::single(doc),
            props(&other, Value::String(istr("not-a-vector"))),
        )
        .expect("non-vector node inserts");
    for &node in &[outgoing_near, outgoing_near] {
        mutator
            .create_edge(support.clone(), root_a, node, PropertyMap::new())
            .expect("root_a support edge inserts");
    }
    for &node in &[outgoing_far, non_vector] {
        mutator
            .create_edge(support.clone(), root_b, node, PropertyMap::new())
            .expect("root_b support edge inserts");
    }
    mutator
        .create_edge(support, incoming, root_a, PropertyMap::new())
        .expect("incoming support edge inserts");
    mutator
        .create_edge(mentions, root_a, wrong_label, PropertyMap::new())
        .expect("wrong-label edge inserts");
    txn.commit().expect("seed graph commits");
    (root_a, root_b, outgoing_near, outgoing_far, incoming)
}

#[test]
fn vector_score_expanded_candidates_accepts_gql_query_roots() {
    let graph = graph(330_223);
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, outgoing_near, outgoing_far, _) = seed_expanded_candidate_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[3.2, 0.0])));

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
fn vector_score_nodes_reranks_explicit_candidates_without_index() {
    let graph = graph(330_207);
    let registry = BuiltinProcedureRegistry::new();
    let ids = seed_vector_graph(&graph);
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .delete_node(ids[7])
            .expect("delete candidate succeeds");
        txn.commit().expect("delete commits");
    }
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[3.2, 0.0])));
    session.bind_parameter(
        istr("nodes"),
        Value::List(vec![
            Value::NodeRef(ids[5]),
            Value::NodeRef(ids[3]),
            Value::NodeRef(ids[3]),
            Value::NodeRef(ids[0]),
            Value::NodeRef(ids[7]),
            Value::NodeRef(ids[8]),
            Value::NodeRef(NodeId::new(999)),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_nodes('embedding', $query, $nodes, 3, 'squared_euclidean') \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![ids[3], ids[5], ids[0]]);
}

#[test]
fn vector_score_nodes_returns_no_rows_for_empty_candidate_list() {
    let graph = graph(330_208);
    let registry = BuiltinProcedureRegistry::new();
    seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(istr("nodes"), Value::List(Vec::new()));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_nodes('embedding', $query, $nodes, 10) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(table.row_count(), 0);
}

#[test]
fn vector_score_nodes_rejects_non_node_candidates() {
    let graph = graph(330_209);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(istr("nodes"), Value::List(vec![Value::Int(1)]));

    let err = session
        .execute_source(
            "CALL selene.vector_score_nodes('embedding', $query, $nodes, 10)",
            &registry,
        )
        .expect_err("non-node candidates must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("nodes[0] must be a NODE")
    ));
}

#[test]
fn vector_score_nodes_batch_reranks_per_query_candidate_sets() {
    let graph = graph(330_210);
    let registry = BuiltinProcedureRegistry::new();
    let ids = seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![
            Value::Vector(vector(&[3.2, 0.0])),
            Value::Vector(vector(&[5.1, 0.0])),
        ]),
    );
    session.bind_parameter(
        istr("nodes"),
        Value::List(vec![
            Value::List(vec![
                Value::NodeRef(ids[5]),
                Value::NodeRef(ids[3]),
                Value::NodeRef(ids[3]),
                Value::NodeRef(ids[0]),
            ]),
            Value::List(vec![
                Value::NodeRef(ids[7]),
                Value::NodeRef(ids[5]),
                Value::NodeRef(ids[1]),
            ]),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_nodes_batch('embedding', $queries, $nodes, 2, 'squared_euclidean') \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    assert_eq!(
        node_column(&table, "node_id"),
        vec![ids[3], ids[5], ids[5], ids[7]]
    );
}

#[test]
fn vector_score_nodes_batch_returns_no_rows_for_empty_queries() {
    let graph = graph(330_211);
    let registry = BuiltinProcedureRegistry::new();
    seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("queries"), Value::List(Vec::new()));
    session.bind_parameter(istr("nodes"), Value::List(Vec::new()));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_nodes_batch('embedding', $queries, $nodes, 10) \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(table.row_count(), 0);
}

#[test]
fn vector_score_nodes_batch_rejects_mismatched_query_and_node_sets() {
    let graph = graph(330_212);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );
    session.bind_parameter(istr("nodes"), Value::List(Vec::new()));

    let err = session
        .execute_source(
            "CALL selene.vector_score_nodes_batch('embedding', $queries, $nodes, 10)",
            &registry,
        )
        .expect_err("mismatched batch arity must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("queries and nodes must have the same length")
    ));
}

#[test]
fn vector_score_nodes_batch_rejects_non_nested_node_candidates() {
    let graph = graph(330_213);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );
    session.bind_parameter(
        istr("nodes"),
        Value::List(vec![Value::List(vec![Value::Int(1)])]),
    );

    let err = session
        .execute_source(
            "CALL selene.vector_score_nodes_batch('embedding', $queries, $nodes, 10)",
            &registry,
        )
        .expect_err("non-node nested candidates must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("nodes[0][0] must be a NODE")
    ));
}

#[test]
fn vector_score_expanded_candidates_scores_preserved_roots_and_outgoing_expansion() {
    let graph = graph(330_217);
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, outgoing_near, outgoing_far, _) = seed_expanded_candidate_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[3.2, 0.0])));
    session.bind_parameter(
        istr("roots"),
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
    session.bind_parameter(istr("query"), Value::Vector(vector(&[1.1, 0.0])));
    session.bind_parameter(
        istr("roots"),
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
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(istr("roots"), Value::List(vec![Value::Int(1)]));

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
        istr("queries"),
        Value::List(vec![
            Value::Vector(vector(&[3.2, 0.0])),
            Value::Vector(vector(&[1.1, 0.0])),
        ]),
    );
    session.bind_parameter(
        istr("roots"),
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
        istr("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );
    session.bind_parameter(istr("roots"), Value::List(Vec::new()));

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
        istr("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );
    session.bind_parameter(
        istr("roots"),
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

#[test]
fn vector_score_neighbors_reranks_directional_graph_candidates() {
    let graph = graph(330_214);
    let registry = BuiltinProcedureRegistry::new();
    let (anchor, _, ids) = seed_neighbor_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[2.2, 0.0])));
    session.bind_parameter(istr("anchor"), Value::NodeRef(anchor));

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
        istr("queries"),
        Value::List(vec![
            Value::Vector(vector(&[2.2, 0.0])),
            Value::Vector(vector(&[6.2, 0.0])),
        ]),
    );
    session.bind_parameter(
        istr("anchors"),
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
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(istr("anchor"), Value::NodeRef(anchor));

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
