//! End-to-end coverage for native vector candidate-scoring built-ins.

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorValue};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
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
        .column_index(db_string(name))
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
        .column_index(db_string(name))
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
    let doc = db_string("VectorDoc");
    let embedding = db_string("embedding");
    let other = db_string("other");
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
                props(&other, Value::String(db_string("not-a-vector"))),
            )
            .expect("non-vector node inserts"),
    );
    txn.commit().expect("seed graph commits");
    ids
}

fn seed_neighbor_vector_graph(graph: &SharedGraph) -> (NodeId, NodeId, Vec<NodeId>) {
    let anchor_label = db_string("Anchor");
    let doc = db_string("VectorDoc");
    let embedding = db_string("embedding");
    let other = db_string("other");
    let depends = db_string("DEPENDS_ON");
    let mentions = db_string("MENTIONS");
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
            props(&other, Value::String(db_string("not-a-vector"))),
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
    let doc = db_string("VectorDoc");
    let root = db_string("VectorRoot");
    let embedding = db_string("embedding");
    let other = db_string("other");
    let support = db_string("SUPPORTS");
    let mentions = db_string("MENTIONS");
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
            props(&other, Value::String(db_string("not-a-vector"))),
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

#[path = "native_vector_score_builtins/expanded.rs"]
mod expanded;
#[path = "native_vector_score_builtins/explicit.rs"]
mod explicit;
#[path = "native_vector_score_builtins/neighbors.rs"]
mod neighbors;
