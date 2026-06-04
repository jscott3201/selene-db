//! End-to-end coverage for IVF native vector built-ins.

use selene_core::{
    GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, VectorValue,
    intern,
};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
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

fn string_column(table: &BindingTable, name: &str) -> Vec<String> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
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

fn float_column(table: &BindingTable, name: &str) -> Vec<f64> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Float(value)) => *value,
            other => panic!("expected float in {name}, got {other:?}"),
        })
        .collect()
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

#[test]
fn create_vector_index_can_register_ivf_metric_kind() {
    let graph = graph(330_151);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0, 0.0]))),
            )
            .expect("vector node inserts");
        txn.commit().expect("seed commits");
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'ivf', NULL, 'cosine')",
            &registry,
        )
        .expect("ivf vector index creation executes");

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(string_column(&table, "kind"), vec!["vector_ivf_cosine(3)"]);
}

#[test]
fn vector_index_stats_reports_ivf_memory_and_cardinality() {
    let graph = graph(330_152);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for value in 0..9 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                )
                .expect("vector node inserts");
        }
        txn.commit().expect("seed commits");
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("ivf vector index creation executes");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[100.0, 0.0]))),
            )
            .expect("post-build vector insert succeeds");
        txn.commit().expect("post-build insert commits");
    }

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_index_stats() \
         YIELD kind, indexed_rows, hnsw_entries, ivf_entries, ivf_live_entries, \
               ivf_deleted_entries, ivf_centroids, ivf_list_count, \
               ivf_non_empty_list_count, ivf_max_list_len, \
               ivf_average_list_len_basis_points, ivf_assigned_entries, \
               ivf_pending_retrain_entries, estimated_index_bytes, estimated_reachable_bytes",
        &registry,
    );

    assert_eq!(table.row_count(), 1);
    assert_eq!(
        string_column(&table, "kind"),
        vec!["vector_ivf_squared_euclidean(2)"]
    );
    assert_eq!(uint_column(&table, "indexed_rows"), vec![10]);
    assert_eq!(uint_column(&table, "hnsw_entries"), vec![0]);
    assert_eq!(uint_column(&table, "ivf_entries"), vec![10]);
    assert_eq!(uint_column(&table, "ivf_live_entries"), vec![10]);
    assert_eq!(uint_column(&table, "ivf_deleted_entries"), vec![0]);
    assert_eq!(uint_column(&table, "ivf_centroids"), vec![3]);
    assert_eq!(uint_column(&table, "ivf_list_count"), vec![3]);
    assert!(uint_column(&table, "ivf_non_empty_list_count")[0] > 0);
    assert!(uint_column(&table, "ivf_max_list_len")[0] > 0);
    assert_eq!(
        uint_column(&table, "ivf_average_list_len_basis_points"),
        vec![33_333]
    );
    assert_eq!(uint_column(&table, "ivf_assigned_entries"), vec![10]);
    assert_eq!(uint_column(&table, "ivf_pending_retrain_entries"), vec![1]);
    assert!(
        uint_column(&table, "estimated_reachable_bytes")[0]
            > uint_column(&table, "estimated_index_bytes")[0]
    );
}

#[test]
fn rebuild_vector_indexes_reclaims_stale_ivf_entries() {
    let graph = graph(330_154);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    let ids = {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for row in 0..24 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(doc.clone()),
                        props(&embedding, Value::Vector(vector(&[row as f32, 0.0]))),
                    )
                    .expect("vector node inserts"),
            );
        }
        txn.commit().expect("seed commits");
        ids
    };

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("ivf vector index creation executes");

    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for (offset, id) in ids.iter().copied().take(4).enumerate() {
            mutator
                .update_node(
                    id,
                    LabelDiff::new([], []).expect("empty label diff"),
                    PropertyDiff::new(
                        [(
                            embedding.clone(),
                            Value::Vector(vector(&[500.0 + offset as f32, 0.0])),
                        )],
                        [],
                    )
                    .expect("property diff"),
                )
                .expect("vector update succeeds");
        }
        for id in ids.iter().copied().skip(4).take(2) {
            mutator.delete_node(id).expect("delete succeeds");
        }
        txn.commit().expect("churn commits");
    }

    let table = execute_rows(
        &mut session,
        "CALL selene.rebuild_vector_indexes() \
         YIELD kind, before_indexed_rows, after_indexed_rows, \
               before_ivf_entries, after_ivf_entries, \
               before_ivf_deleted_entries, after_ivf_deleted_entries, \
               before_ivf_assigned_entries, after_ivf_centroids, after_ivf_list_count, \
               before_ivf_pending_retrain_entries, after_ivf_pending_retrain_entries, \
               after_ivf_non_empty_list_count, after_ivf_max_list_len, \
               after_ivf_average_list_len_basis_points, after_ivf_assigned_entries, \
               reclaimed_ivf_entries, reclaimed_ivf_deleted_entries, reclaimed_reachable_bytes",
        &registry,
    );

    assert_eq!(table.row_count(), 1);
    assert_eq!(
        string_column(&table, "kind"),
        vec!["vector_ivf_squared_euclidean(2)"]
    );
    assert_eq!(uint_column(&table, "before_indexed_rows"), vec![22]);
    assert_eq!(uint_column(&table, "after_indexed_rows"), vec![22]);
    assert_eq!(uint_column(&table, "before_ivf_entries"), vec![24]);
    assert_eq!(uint_column(&table, "after_ivf_entries"), vec![22]);
    assert_eq!(uint_column(&table, "before_ivf_deleted_entries"), vec![2]);
    assert_eq!(uint_column(&table, "after_ivf_deleted_entries"), vec![0]);
    assert_eq!(uint_column(&table, "before_ivf_assigned_entries"), vec![22]);
    assert_eq!(
        uint_column(&table, "before_ivf_pending_retrain_entries"),
        vec![4]
    );
    assert_eq!(
        uint_column(&table, "after_ivf_pending_retrain_entries"),
        vec![0]
    );
    assert!(uint_column(&table, "after_ivf_centroids")[0] > 0);
    assert!(uint_column(&table, "after_ivf_list_count")[0] > 0);
    assert!(uint_column(&table, "after_ivf_non_empty_list_count")[0] > 0);
    assert!(uint_column(&table, "after_ivf_max_list_len")[0] > 0);
    assert_eq!(
        uint_column(&table, "after_ivf_average_list_len_basis_points")[0],
        uint_column(&table, "after_ivf_assigned_entries")[0] * 10_000
            / uint_column(&table, "after_ivf_list_count")[0]
    );
    assert_eq!(uint_column(&table, "after_ivf_assigned_entries"), vec![22]);
    assert_eq!(uint_column(&table, "reclaimed_ivf_entries"), vec![2]);
    assert_eq!(
        uint_column(&table, "reclaimed_ivf_deleted_entries"),
        vec![2]
    );
}

#[test]
fn vector_search_nodes_ann_uses_registered_ivf_index() {
    let graph = graph(330_153);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");

    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            for i in 0..32 {
                mutator
                    .create_node(
                        LabelSet::single(doc.clone()),
                        props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                    )
                    .expect("vector node inserts");
            }
        }
        txn.commit().expect("seed graph commits");
    }
    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("ivf vector index creation executes");

    session.bind_parameter(istr("query"), Value::Vector(vector(&[4.1, 0.0])));
    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 3, 'squared_euclidean', 64) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(5), NodeId::new(6), NodeId::new(4)]
    );
    let distances = float_column(&table, "distance");
    assert!(distances[0] < distances[1]);
    assert!(distances[1] < distances[2]);
}

#[test]
fn vector_search_nodes_ann_uses_ivf_default_when_width_is_omitted_or_null() {
    let graph = graph(330_155);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");

    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for i in 0..32 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts");
        }
        txn.commit().expect("seed graph commits");
    }
    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("ivf vector index creation executes");

    session.bind_parameter(istr("query"), Value::Vector(vector(&[4.1, 0.0])));
    let omitted = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 3) \
         YIELD node_id, distance",
        &registry,
    );
    let null_width = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 3, 'squared_euclidean', NULL) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(node_column(&omitted, "node_id")[0], NodeId::new(5));
    assert_eq!(
        node_column(&omitted, "node_id"),
        node_column(&null_width, "node_id")
    );
    assert_eq!(
        float_column(&omitted, "distance"),
        float_column(&null_width, "distance")
    );
}

#[test]
fn vector_search_nodes_ann_batch_uses_ivf_default_when_width_is_omitted() {
    let graph = graph(330_156);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");

    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for i in 0..32 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts");
        }
        txn.commit().expect("seed graph commits");
    }
    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("ivf vector index creation executes");
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![
            Value::Vector(vector(&[4.1, 0.0])),
            Value::Vector(vector(&[12.2, 0.0])),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann_batch('VectorDoc', 'embedding', $queries, 2) \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    let nodes = node_column(&table, "node_id");
    assert_eq!(nodes[0], NodeId::new(5));
    assert_eq!(nodes[2], NodeId::new(13));
}
