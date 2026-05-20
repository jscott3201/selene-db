//! `vector.ivf_search` and `vector.ivf_stats` adapter tests.

use std::collections::BTreeSet;
use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ExecutorError, ImplDefinedCaps, MutationContext, ProcedureContext,
    ProcedureError, ProcedureRegistry, ProcedureResult, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{DistanceMetric, IvfConfig, IvfProvider, PqParams};
use selene_vector_pack::VectorPack;

const TRAINING_ROWS: usize = 256;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn registry(pack: &VectorPack) -> ProcedurePackRegistry {
    pack.registry_with_builtins()
        .expect("vector pack registers cleanly")
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

fn execute_result(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source, registry);
    let mut session = selene_gql::Session::new(graph);
    execute_statement(&plan, &mut session, registry)
}

fn execute_ok(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
) -> StatementOutput {
    execute_result(source, graph, registry).expect("statement executes")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_mutation_direct(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    name: &[&str],
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    let interned = name.iter().map(|segment| istr(segment)).collect::<Vec<_>>();
    let metadata = registry
        .lookup(&interned)
        .expect("mutation procedure registered");
    let mut txn = graph.begin_write();
    let caps = ImplDefinedCaps::default();
    let result = {
        let mut ctx = ProcedureContext::Mutation(MutationContext::for_test(txn.mutator(), &caps));
        registry.execute(metadata.handle, args, &mut ctx)
    };
    match result {
        Ok(result) => {
            txn.commit().expect("mutation commit succeeds");
            Ok(result)
        }
        Err(err) => {
            txn.rollback();
            Err(err)
        }
    }
}

fn graph_with_ivf_nodes(id: u64, labels: &[&str]) -> (SharedGraph, Arc<IvfProvider>, Vec<NodeId>) {
    let provider = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let node_ids = create_nodes(&graph, labels);
    (graph, provider, node_ids)
}

fn create_nodes(graph: &SharedGraph, labels: &[&str]) -> Vec<NodeId> {
    let mut node_ids = Vec::with_capacity(labels.len());
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        for label in labels {
            node_ids.push(
                mutator
                    .create_node(LabelSet::single(istr(label)), PropertyMap::new())
                    .expect("fixture node inserts"),
            );
        }
    }
    txn.commit().expect("fixture commit succeeds");
    node_ids
}

fn ivf_config() -> IvfConfig {
    IvfConfig::with_params(
        2,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: TRAINING_ROWS,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        TRAINING_ROWS,
    )
    .unwrap()
}

fn seed_rows(count: usize) -> (Vec<&'static str>, Vec<[f64; 2]>) {
    let labels = (0..count)
        .map(|idx| if idx < 2 { "Allowed" } else { "Vec" })
        .collect();
    let vectors = (0..count)
        .map(|idx| [idx as f64, (idx % 11) as f64])
        .collect();
    (labels, vectors)
}

fn node_ref_list(node_ids: &[NodeId]) -> Value {
    Value::List(node_ids.iter().copied().map(Value::NodeRef).collect())
}

fn vector_matrix(rows: &[[f64; 2]]) -> Value {
    Value::List(
        rows.iter()
            .map(|row| Value::List(row.iter().copied().map(Value::Float).collect()))
            .collect(),
    )
}

fn bulk_upsert_ivf(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    nodes: &[NodeId],
    vectors: &[[f64; 2]],
) {
    execute_mutation_direct(
        graph,
        registry,
        &["vector", "ivf_bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(nodes),
            vector_matrix(vectors),
        ],
    )
    .expect("IVF bulk upsert succeeds");
}

fn trained_fixture(id: u64) -> (SharedGraph, Arc<IvfProvider>, Vec<NodeId>) {
    let (labels, vectors) = seed_rows(TRAINING_ROWS);
    let (graph, provider, nodes) = graph_with_ivf_nodes(id, &labels);
    let pack = VectorPack::new();
    let registry = registry(&pack);
    bulk_upsert_ivf(&graph, &registry, &nodes, &vectors);
    train_provider(&provider);
    (graph, provider, nodes)
}

fn train_provider(provider: &IvfProvider) {
    provider
        .write_section(SubTag(*b"CQNT"))
        .expect("CQNT write trains");
    provider
        .write_section(SubTag(*b"IPQB"))
        .expect("IPQB write publishes codebook");
    provider
        .write_section(SubTag(*b"POST"))
        .expect("POST write publishes postings");
}

fn node_ids(table: &BindingTable) -> Vec<NodeId> {
    let index = table
        .schema()
        .columns
        .iter()
        .position(|column| {
            column
                .name
                .is_some_and(|column| column.as_str() == "node_id")
        })
        .expect("column exists");
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(node_id)) => *node_id,
            other => panic!("expected node ref, got {other:?}"),
        })
        .collect()
}

fn string_value(value: &Value) -> &str {
    match value {
        Value::String(value) => value.as_str(),
        Value::ExternalString(value) => value,
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn ivf_search_returns_top_k_after_training() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, provider, _) = trained_fixture(88_001);

    let table = rows(execute_ok(
        "CALL vector.ivf_search('default', [0.0, 0.0], 5, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected = provider
        .search(&[0.0, 0.0], 5, None, None, None)
        .expect("direct search succeeds")
        .into_iter()
        .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(f64::from(score))])
        .collect::<Vec<_>>();

    assert_eq!(
        table
            .rows()
            .iter()
            .map(|row| row.values().to_vec())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(table.rows().len(), 5);
}

#[test]
fn ivf_search_below_threshold_untrained_returns_empty() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (labels, vectors) = seed_rows(3);
    let (graph, _, nodes) = graph_with_ivf_nodes(88_002, &labels);
    bulk_upsert_ivf(&graph, &registry, &nodes, &vectors);

    let table = rows(execute_ok(
        "CALL vector.ivf_search('default', [0.0, 0.0], 5, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert!(table.is_empty());
}

#[test]
fn ivf_search_at_threshold_untrained_returns_empty_without_snapshot_writes() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (labels, vectors) = seed_rows(TRAINING_ROWS);
    let (graph, _, nodes) = graph_with_ivf_nodes(88_003, &labels);
    bulk_upsert_ivf(&graph, &registry, &nodes, &vectors);

    let table = rows(execute_ok(
        "CALL vector.ivf_search('default', [0.0, 0.0], 5, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert!(table.is_empty());
}

#[test]
fn ivf_search_accepts_n_probe_override() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = trained_fixture(88_004);

    let one_probe = rows(execute_ok(
        "CALL vector.ivf_search('default', [0.0, 0.0], 5, 1, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let all_probes = rows(execute_ok(
        "CALL vector.ivf_search('default', [0.0, 0.0], 5, 4, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert!(one_probe.rows().len() <= 5);
    assert!(all_probes.rows().len() <= 5);
}

#[test]
fn ivf_search_rejects_out_of_range_n_probe_as_invalid_argument() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = trained_fixture(88_005);

    let err = execute_result(
        "CALL vector.ivf_search('default', [0.0, 0.0], 5, 0, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("bad n_probe rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_search_filter_narrows_to_filtered_set() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = trained_fixture(88_006);

    let table = rows(execute_ok(
        "MATCH (n:Allowed) WITH collect(n) AS nodes \
         CALL vector.ivf_search('default', [0.0, 0.0], 10, 4, nodes, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let allowed = nodes[..2].iter().copied().collect::<BTreeSet<_>>();

    assert!(!table.is_empty());
    assert!(
        node_ids(&table)
            .into_iter()
            .all(|node| allowed.contains(&node))
    );
}

#[test]
fn ivf_search_rejects_non_default_index() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = trained_fixture(88_007);

    let err = execute_result(
        "CALL vector.ivf_search('embedding_idx', [0.0, 0.0], 5, NULL, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("unknown index rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_stats_trained_emits_one_row_with_trained_fields() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = trained_fixture(88_008);

    let table = rows(execute_ok(
        "CALL vector.ivf_stats('default') YIELD *",
        &graph,
        &registry,
    ));
    let row = table.rows()[0].values();

    assert_eq!(table.rows().len(), 1);
    assert_eq!(string_value(&row[0]), "trained");
    assert_eq!(row[1], Value::Int(4));
    assert!(matches!(row[3], Value::List(_)));
    assert_eq!(row[12], Value::Null);
    assert_eq!(row[13], Value::Null);
}

#[test]
fn ivf_stats_deferred_emits_one_row_with_deferred_fields() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (labels, vectors) = seed_rows(3);
    let (graph, _, nodes) = graph_with_ivf_nodes(88_009, &labels);
    bulk_upsert_ivf(&graph, &registry, &nodes, &vectors);

    let table = rows(execute_ok(
        "CALL vector.ivf_stats('default') YIELD *",
        &graph,
        &registry,
    ));
    let row = table.rows()[0].values();

    assert_eq!(table.rows().len(), 1);
    assert_eq!(string_value(&row[0]), "deferred");
    assert!(row[1..12].iter().all(|value| *value == Value::Null));
    assert_eq!(row[12], Value::Int(3));
    assert_eq!(row[13], Value::Int(TRAINING_ROWS as i64));
}

#[test]
fn ivf_stats_empty_index_emits_zero_rows() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = graph_with_ivf_nodes(88_010, &[]);

    let table = rows(execute_ok(
        "CALL vector.ivf_stats('default') YIELD *",
        &graph,
        &registry,
    ));

    assert!(table.is_empty());
}

#[test]
fn ivf_stats_above_threshold_untrained_emits_zero_rows() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (labels, vectors) = seed_rows(TRAINING_ROWS);
    let (graph, _, nodes) = graph_with_ivf_nodes(88_011, &labels);
    bulk_upsert_ivf(&graph, &registry, &nodes, &vectors);

    let table = rows(execute_ok(
        "CALL vector.ivf_stats('default') YIELD *",
        &graph,
        &registry,
    ));

    assert!(table.is_empty());
}

#[test]
fn ivf_stats_rejects_non_default_index() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = graph_with_ivf_nodes(88_012, &[]);

    let err = execute_result(
        "CALL vector.ivf_stats('embedding_idx') YIELD state",
        &graph,
        &registry,
    )
    .expect_err("unknown index rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}
