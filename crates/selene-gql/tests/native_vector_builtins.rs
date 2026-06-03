//! End-to-end coverage for native `selene.*` vector built-ins.

use selene_core::{
    GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, VectorValue,
    intern,
};
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
fn create_vector_index_commits_through_the_funnel() {
    let graph = graph(330_013);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 2.0, 3.0]))),
            )
            .expect("vector node inserts");
        txn.commit().expect("seed commits");
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3)",
            &registry,
        )
        .expect("vector index creation executes");

    let snapshot = graph.read();
    let index = snapshot
        .vector_index_for(&doc, &embedding)
        .expect("vector index is committed");
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    drop(snapshot);

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(string_column(&table, "label"), vec!["VectorDoc"]);
    assert_eq!(string_column(&table, "property"), vec!["embedding"]);
    assert_eq!(string_column(&table, "kind"), vec!["vector_flat(3)"]);
}

#[test]
fn create_vector_index_can_register_hnsw_metric_kind() {
    let graph = graph(330_015);
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
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'hnsw', NULL, 'cosine')",
            &registry,
        )
        .expect("hnsw vector index creation executes");

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(string_column(&table, "kind"), vec!["vector_hnsw_cosine(3)"]);
}

#[test]
fn vector_index_stats_reports_hnsw_memory_and_cardinality() {
    let graph = graph(330_016);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for components in [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]] {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&components))),
                )
                .expect("vector node inserts");
        }
        txn.commit().expect("seed commits");
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'hnsw', NULL, NULL, 24, 64)",
            &registry,
        )
        .expect("hnsw vector index creation executes");

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_index_stats() YIELD name, label, property, kind, dimension, indexed_rows, hnsw_entries, hnsw_live_entries, hnsw_deleted_entries, hnsw_link_count, hnsw_level_zero_link_count, hnsw_upper_layer_link_count, hnsw_max_layer_count, hnsw_max_links_per_layer, hnsw_average_links_per_entry_basis_points, estimated_index_bytes, estimated_reachable_bytes",
        &registry,
    );
    assert_eq!(table.row_count(), 1);
    assert_eq!(string_column(&table, "label"), vec!["VectorDoc"]);
    assert_eq!(string_column(&table, "property"), vec!["embedding"]);
    assert_eq!(
        string_column(&table, "kind"),
        vec!["vector_hnsw_squared_euclidean(3,m=24,ef_construction=64)"]
    );
    assert_eq!(uint_column(&table, "dimension"), vec![3]);
    assert_eq!(uint_column(&table, "indexed_rows"), vec![2]);
    assert_eq!(uint_column(&table, "hnsw_entries"), vec![2]);
    assert_eq!(uint_column(&table, "hnsw_live_entries"), vec![2]);
    assert_eq!(uint_column(&table, "hnsw_deleted_entries"), vec![0]);
    assert!(uint_column(&table, "hnsw_link_count")[0] > 0);
    assert!(uint_column(&table, "hnsw_level_zero_link_count")[0] > 0);
    assert_eq!(
        uint_column(&table, "hnsw_level_zero_link_count")[0]
            + uint_column(&table, "hnsw_upper_layer_link_count")[0],
        uint_column(&table, "hnsw_link_count")[0]
    );
    assert!(uint_column(&table, "hnsw_max_layer_count")[0] > 0);
    assert!(uint_column(&table, "hnsw_max_links_per_layer")[0] > 0);
    assert!(uint_column(&table, "hnsw_average_links_per_entry_basis_points")[0] > 0);
    assert!(uint_column(&table, "estimated_index_bytes")[0] > 0);
    assert!(
        uint_column(&table, "estimated_reachable_bytes")[0]
            > uint_column(&table, "estimated_index_bytes")[0]
    );
    assert_eq!(
        string_column(&table, "name"),
        vec!["vidx:9:VectorDoc:9:embedding"]
    );
}

#[test]
fn rebuild_vector_indexes_reclaims_stale_hnsw_entries() {
    let graph = graph(330_019);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    let ids = {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for row in 0..48 {
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
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'hnsw')",
            &registry,
        )
        .expect("hnsw vector index creation executes");

    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for (offset, id) in ids.iter().copied().take(8).enumerate() {
            mutator
                .update_node(
                    id,
                    LabelDiff::new([], []).expect("empty label diff"),
                    PropertyDiff::new(
                        [(
                            embedding.clone(),
                            Value::Vector(vector(&[1_000.0 + offset as f32, 0.0])),
                        )],
                        [],
                    )
                    .expect("property diff"),
                )
                .expect("vector update succeeds");
        }
        for id in ids.iter().copied().skip(8).take(4) {
            mutator.delete_node(id).expect("delete succeeds");
        }
        txn.commit().expect("churn commits");
    }

    let table = execute_rows(
        &mut session,
        "CALL selene.rebuild_vector_indexes() \
         YIELD label, property, kind, before_indexed_rows, after_indexed_rows, \
               before_hnsw_entries, after_hnsw_entries, \
               before_hnsw_deleted_entries, after_hnsw_deleted_entries, \
               before_hnsw_link_count, after_hnsw_link_count, \
               before_hnsw_level_zero_link_count, after_hnsw_level_zero_link_count, \
               before_hnsw_upper_layer_link_count, after_hnsw_upper_layer_link_count, \
               after_hnsw_max_layer_count, after_hnsw_max_links_per_layer, \
               after_hnsw_average_links_per_entry_basis_points, \
               reclaimed_hnsw_entries, reclaimed_hnsw_deleted_entries, \
               reclaimed_reachable_bytes",
        &registry,
    );

    assert_eq!(table.row_count(), 1);
    assert_eq!(string_column(&table, "label"), vec!["VectorDoc"]);
    assert_eq!(string_column(&table, "property"), vec!["embedding"]);
    assert_eq!(
        string_column(&table, "kind"),
        vec!["vector_hnsw_squared_euclidean(2)"]
    );
    assert_eq!(uint_column(&table, "before_indexed_rows"), vec![44]);
    assert_eq!(uint_column(&table, "after_indexed_rows"), vec![44]);
    assert_eq!(uint_column(&table, "before_hnsw_entries"), vec![56]);
    assert_eq!(uint_column(&table, "after_hnsw_entries"), vec![44]);
    assert_eq!(uint_column(&table, "before_hnsw_deleted_entries"), vec![12]);
    assert_eq!(uint_column(&table, "after_hnsw_deleted_entries"), vec![0]);
    assert_eq!(
        uint_column(&table, "after_hnsw_level_zero_link_count")[0]
            + uint_column(&table, "after_hnsw_upper_layer_link_count")[0],
        uint_column(&table, "after_hnsw_link_count")[0]
    );
    assert_eq!(
        uint_column(&table, "before_hnsw_level_zero_link_count")[0]
            + uint_column(&table, "before_hnsw_upper_layer_link_count")[0],
        uint_column(&table, "before_hnsw_link_count")[0]
    );
    assert!(uint_column(&table, "after_hnsw_level_zero_link_count")[0] > 0);
    assert!(uint_column(&table, "after_hnsw_max_layer_count")[0] > 0);
    assert!(uint_column(&table, "after_hnsw_max_links_per_layer")[0] > 0);
    assert!(uint_column(&table, "after_hnsw_average_links_per_entry_basis_points")[0] > 0);
    assert_eq!(uint_column(&table, "reclaimed_hnsw_entries"), vec![12]);
    assert_eq!(
        uint_column(&table, "reclaimed_hnsw_deleted_entries"),
        vec![12]
    );
    assert!(uint_column(&table, "reclaimed_reachable_bytes")[0] > 0);
}

#[test]
fn rebuild_vector_indexes_is_rejected_inside_explicit_tx() {
    let graph = graph(330_020);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source("START TRANSACTION", &registry)
        .expect("transaction starts");
    let err = session
        .execute_source("CALL selene.rebuild_vector_indexes()", &registry)
        .expect_err("maintenance call inside explicit transaction is rejected");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState { detail, .. }
            if detail.contains("maintenance procedure cannot run inside an explicit transaction")
    ));
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn vector_search_nodes_returns_exact_matches_from_vector_parameter() {
    let graph = graph(330_011);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let other = istr("VectorOther");
    let embedding = istr("embedding");

    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[2.0, 0.0]))),
                )
                .expect("first vector node inserts");
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
                )
                .expect("second vector node inserts");
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::String(istr("skip"))),
                )
                .expect("non-vector property node inserts");
            mutator
                .create_node(
                    LabelSet::single(other),
                    props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
                )
                .expect("other label vector node inserts");
        }
        txn.commit().expect("seed graph commits");
    }

    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes('VectorDoc', 'embedding', $query, 10) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(2), NodeId::new(1)]
    );
    assert_eq!(float_column(&table, "distance"), vec![1.0, 4.0]);
}

#[test]
fn vector_search_nodes_ann_uses_registered_hnsw_index() {
    let graph = graph(330_016);
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
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'hnsw')",
            &registry,
        )
        .expect("hnsw vector index creation executes");

    session.bind_parameter(istr("query"), Value::Vector(vector(&[4.1, 0.0])));
    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 3, 'squared_euclidean', 32) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id")[0], NodeId::new(5));
    assert_eq!(table.row_count(), 3);
}

#[test]
fn vector_search_nodes_ann_requires_hnsw_index() {
    let graph = graph(330_017);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 10)",
            &registry,
        )
        .expect_err("missing hnsw index must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("requires a matching HNSW vector index")
    ));
}

#[test]
fn vector_search_nodes_query_parameter_must_be_vector() {
    let graph = graph(330_012);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session.bind_parameter(istr("query"), Value::List(vec![Value::Float(0.0)]));
    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes('VectorDoc', 'embedding', $query, 10)",
            &registry,
        )
        .expect_err("non-vector query parameter must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("query must be a VECTOR")
    ));
}

#[test]
fn vector_search_nodes_ann_reports_metric_mismatch() {
    let graph = graph(330_018);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
            )
            .expect("vector node inserts");
        txn.commit().expect("seed commits");
    }
    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'hnsw', NULL, 'cosine')",
            &registry,
        )
        .expect("hnsw vector index creation executes");

    session.bind_parameter(istr("query"), Value::Vector(vector(&[1.0, 0.0])));
    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 10, 'squared_euclidean')",
            &registry,
        )
        .expect_err("wrong metric must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("HNSW vector index uses Cosine")
    ));
}
