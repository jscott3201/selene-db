//! Metric override coverage for `vector.search` and `vector.ivf_search`.

use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ExecutorError, ImplDefinedCaps, MutationContext, ProcedureContext,
    ProcedureError, ProcedureRegistry, ProcedureResult, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{DistanceMetric, HnswConfig, HnswProvider, IvfConfig, IvfProvider, PqParams};
use selene_vector_pack::VectorPack;

const IVF_ROWS: usize = 256;

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

fn hnsw_config(metric: DistanceMetric) -> HnswConfig {
    HnswConfig::with_params(2, 16, 200, 50, metric).expect("HNSW config valid")
}

fn hnsw_fixture(id: u64, metric: DistanceMetric) -> SharedGraph {
    let provider = Arc::new(HnswProvider::new(hnsw_config(metric)).unwrap());
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let pack = VectorPack::new();
    let registry = registry(&pack);
    for (offset, vector) in [[10.0, 0.0], [1.0, 1.0], [0.0, 10.0], [-1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        let node_id = create_node(&graph, offset as u64);
        execute_mutation_direct(
            &graph,
            &registry,
            &["vector", "upsert"],
            &[
                Value::String(istr("default")),
                Value::NodeRef(node_id),
                Value::List(vector.into_iter().map(Value::Float).collect()),
            ],
        )
        .expect("upsert succeeds");
    }
    graph
}

fn create_node(graph: &SharedGraph, offset: u64) -> NodeId {
    let mut txn = graph.begin_write();
    let node_id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Vector")),
                PropertyMap::from_pairs([(istr("offset"), Value::Int(offset as i64))])
                    .expect("properties valid"),
            )
            .expect("fixture node inserts")
    };
    txn.commit().expect("fixture commit succeeds");
    node_id
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
        .expect("node_id column exists");
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(node_id)) => *node_id,
            other => panic!("expected node ref, got {other:?}"),
        })
        .collect()
}

fn metric_name(metric: DistanceMetric) -> &'static str {
    match metric {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::L2 => "l2",
        DistanceMetric::Dot => "dot",
        _ => "unknown",
    }
}

fn ivf_config(metric: DistanceMetric) -> IvfConfig {
    IvfConfig::with_params(
        2,
        4,
        2,
        metric,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: IVF_ROWS,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        IVF_ROWS,
    )
    .expect("IVF config valid")
}

fn ivf_fixture(id: u64, metric: DistanceMetric) -> (SharedGraph, Arc<IvfProvider>) {
    let provider = Arc::new(IvfProvider::new(ivf_config(metric)).unwrap());
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let mut node_ids = Vec::with_capacity(IVF_ROWS);
    for offset in 0..IVF_ROWS {
        node_ids.push(create_node(&graph, offset as u64));
    }
    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "ivf_bulk_upsert"],
        &[
            Value::String(istr("default")),
            Value::List(node_ids.into_iter().map(Value::NodeRef).collect()),
            Value::List(
                (0..IVF_ROWS)
                    .map(|idx| {
                        Value::List(vec![
                            Value::Float(idx as f64),
                            Value::Float((idx % 17) as f64),
                        ])
                    })
                    .collect(),
            ),
        ],
    )
    .expect("IVF bulk upsert succeeds");
    train_provider(&provider);
    (graph, provider)
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

#[test]
fn hnsw_metric_override_changes_query_time_scoring() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = hnsw_fixture(119_001, DistanceMetric::Cosine);

    let cosine = node_ids(&rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0], 2, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    )));
    let l2 = node_ids(&rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0], 2, NULL, NULL, 'l2') YIELD node_id, score",
        &graph,
        &registry,
    )));

    assert_ne!(cosine, l2);
}

#[test]
fn hnsw_metric_override_all_build_query_pairs_execute() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let metrics = [
        DistanceMetric::Cosine,
        DistanceMetric::L2,
        DistanceMetric::Dot,
    ];

    for (index, build_metric) in metrics.into_iter().enumerate() {
        let graph = hnsw_fixture(119_010 + index as u64, build_metric);
        for query_metric in metrics {
            let source = format!(
                "CALL vector.search('default', [1.0, 0.0], 2, NULL, NULL, '{}') YIELD node_id, score",
                metric_name(query_metric)
            );
            let table = rows(execute_ok(&source, &graph, &registry));
            assert!(!table.rows().is_empty());
        }
    }
}

#[test]
fn search_metric_parser_accepts_case_and_null_rejects_unknown() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = hnsw_fixture(119_020, DistanceMetric::Cosine);

    for metric in ["cosine", "COSINE", "l2", "L2", "dot", "Dot"] {
        let source = format!(
            "CALL vector.search('default', [1.0, 0.0], 1, NULL, NULL, '{metric}') YIELD node_id"
        );
        rows(execute_ok(&source, &graph, &registry));
    }
    rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0], 1, NULL, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    ));

    let err = execute_result(
        "CALL vector.search('default', [1.0, 0.0], 1, NULL, NULL, 'manhattan') YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("unknown metric rejected");
    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_metric_override_allowed_pairs_execute() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let metrics = [
        DistanceMetric::Cosine,
        DistanceMetric::L2,
        DistanceMetric::Dot,
    ];

    for (index, build_metric) in metrics.into_iter().enumerate() {
        let (graph, _) = ivf_fixture(119_030 + index as u64, build_metric);
        for query_metric in metrics {
            if query_metric == DistanceMetric::Cosine && build_metric != DistanceMetric::Cosine {
                continue;
            }
            let source = format!(
                "CALL vector.ivf_search('default', [1.0, 0.0], 5, NULL, NULL, '{}') YIELD node_id, score",
                metric_name(query_metric)
            );
            let table = rows(execute_ok(&source, &graph, &registry));
            assert!(!table.rows().is_empty());
        }
    }
}

#[test]
fn ivf_cosine_override_requires_cosine_side_data() {
    let pack = VectorPack::new();
    let registry = registry(&pack);

    for (index, build_metric) in [DistanceMetric::L2, DistanceMetric::Dot]
        .into_iter()
        .enumerate()
    {
        let (graph, _) = ivf_fixture(119_040 + index as u64, build_metric);
        let err = execute_result(
            "CALL vector.ivf_search('default', [1.0, 0.0], 5, NULL, NULL, 'cosine') YIELD node_id",
            &graph,
            &registry,
        )
        .expect_err("cosine override rejected without side data");
        let ExecutorError::Procedure { source, .. } = err else {
            panic!("expected procedure error");
        };
        assert!(matches!(source, ProcedureError::InvalidArgument { .. }));
        assert_eq!(source.gqlstatus().as_str(), "22G03");
    }
}
