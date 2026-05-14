//! `vector.*bulk*` adapter tests.

use std::sync::Arc;

use selene_core::{Change, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ImplDefinedCaps, MutationContext, ProcedureContext,
    ProcedureError, ProcedureRegistry, ProcedureResult, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, SharedGraph};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{DistanceMetric, HnswConfig, HnswProvider, IvfConfig, IvfProvider, PqParams};
use selene_vector_pack::VectorPack;

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

fn execute_ok(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
) -> StatementOutput {
    let plan = planned(source, registry);
    let mut session = selene_gql::Session::new(graph);
    execute_statement(&plan, &mut session, registry).expect("statement executes")
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

fn execute_mutation_batch_direct(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    calls: &[(&[&str], Vec<Value>)],
) -> Result<Vec<Change>, ProcedureError> {
    let mut txn = graph.begin_write();
    let caps = ImplDefinedCaps::default();
    let mut result = Ok(());
    {
        let mut ctx = ProcedureContext::Mutation(MutationContext::for_test(txn.mutator(), &caps));
        for (name, args) in calls {
            let interned = name.iter().map(|segment| istr(segment)).collect::<Vec<_>>();
            let metadata = registry
                .lookup(&interned)
                .expect("mutation procedure registered");
            if let Err(err) = registry.execute(metadata.handle, args, &mut ctx) {
                result = Err(err);
                break;
            }
        }
    }
    match result {
        Ok(()) => Ok(txn.commit().expect("mutation commit succeeds").changes),
        Err(err) => {
            txn.rollback();
            Err(err)
        }
    }
}

fn graph_with_nodes(id: u64, labels: &[&str]) -> (SharedGraph, Arc<HnswProvider>, Vec<NodeId>) {
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let node_ids = create_nodes(&graph, labels);
    (graph, provider, node_ids)
}

fn graph_with_hnsw_and_ivf_nodes(
    id: u64,
    labels: &[&str],
) -> (
    SharedGraph,
    Arc<HnswProvider>,
    Arc<IvfProvider>,
    Vec<NodeId>,
) {
    let hnsw = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(hnsw.clone() as Arc<dyn IndexProvider>)
        .with_provider(ivf.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let node_ids = create_nodes(&graph, labels);
    (graph, hnsw, ivf, node_ids)
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
        4,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        256,
    )
    .unwrap()
}

fn node_ref_list(node_ids: &[NodeId]) -> Value {
    Value::List(node_ids.iter().copied().map(Value::NodeRef).collect())
}

fn vector_matrix(rows: &[&[f64]]) -> Value {
    Value::List(
        rows.iter()
            .map(|row| Value::List(row.iter().copied().map(Value::Float).collect()))
            .collect(),
    )
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

fn index_extension_event_count(changes: &[Change], provider_name: &str) -> usize {
    changes
        .iter()
        .filter(|change| {
            matches!(
                change, Change::IndexExtensionEvent { provider, .. }
                if *provider == istr(provider_name)
            )
        })
        .count()
}

#[test]
fn vector_bulk_upsert_inserts_rows_and_search_returns_them() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(87_901, &["Vec", "Vec"]);

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&nodes),
            vector_matrix(&[&[1.0, 0.0, 0.0, 0.0], &[0.9, 0.1, 0.0, 0.0]]),
        ],
    )
    .expect("bulk upsert succeeds");
    let table = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 2, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let mut observed = node_ids(&table);
    observed.sort_unstable();
    let mut expected = nodes;
    expected.sort_unstable();

    assert_eq!(observed, expected);
}

#[test]
fn vector_bulk_delete_removes_multiple_vectors() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(87_902, &["Vec", "Vec", "Vec"]);
    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&nodes),
            vector_matrix(&[
                &[1.0, 0.0, 0.0, 0.0],
                &[0.0, 1.0, 0.0, 0.0],
                &[0.0, 0.0, 1.0, 0.0],
            ]),
        ],
    )
    .expect("bulk upsert succeeds");

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "bulk_delete"],
        &[
            Value::String(istr("default")),
            node_ref_list(&[nodes[0], nodes[1]]),
        ],
    )
    .expect("bulk delete succeeds");
    let table = rows(execute_ok(
        "CALL vector.search('default', [0.0, 0.0, 1.0, 0.0], 3, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(node_ids(&table), vec![nodes[2]]);
}

#[test]
fn vector_bulk_upsert_rejects_parallel_length_mismatch() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(87_903, &["Vec", "Vec"]);

    let err = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&nodes),
            vector_matrix(&[&[1.0, 0.0, 0.0, 0.0]]),
        ],
    )
    .expect_err("parallel mismatch rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_bulk_upsert_rejects_tombstone_and_duplicate_rows() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(87_904, &["Vec"]);

    let tombstone = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&[NodeId::TOMBSTONE]),
            vector_matrix(&[&[1.0, 0.0, 0.0, 0.0]]),
        ],
    )
    .expect_err("tombstone rejected");
    let duplicate = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&[nodes[0], nodes[0]]),
            vector_matrix(&[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]]),
        ],
    )
    .expect_err("duplicate rejected");

    assert!(matches!(tombstone, ProcedureError::InvalidArgument { .. }));
    assert!(matches!(duplicate, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_bulk_upsert_emits_one_event_per_call() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(87_905, &["Vec", "Vec"]);

    let changes = execute_mutation_batch_direct(
        &graph,
        &registry,
        &[(
            &["vector", "bulk_upsert"],
            vec![
                Value::String(istr("default")),
                node_ref_list(&nodes),
                vector_matrix(&[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]]),
            ],
        )],
    )
    .expect("bulk upsert succeeds");

    assert_eq!(index_extension_event_count(&changes, "selene-vector"), 1);
}

#[test]
fn vector_ivf_bulk_upsert_appends_deferred_rows() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, hnsw, ivf, nodes) = graph_with_hnsw_and_ivf_nodes(87_906, &["Vec", "Vec"]);

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "ivf_bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&nodes),
            vector_matrix(&[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]]),
        ],
    )
    .expect("IVF bulk upsert succeeds");

    assert_eq!(ivf.snapshot().len(), 2);
    assert_eq!(hnsw.snapshot().len(), 0);
}

#[test]
fn vector_ivf_bulk_delete_removes_deferred_rows() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, ivf, nodes) = graph_with_hnsw_and_ivf_nodes(87_907, &["Vec", "Vec", "Vec"]);
    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "ivf_bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&nodes),
            vector_matrix(&[
                &[1.0, 0.0, 0.0, 0.0],
                &[0.0, 1.0, 0.0, 0.0],
                &[0.0, 0.0, 1.0, 0.0],
            ]),
        ],
    )
    .expect("IVF bulk upsert succeeds");

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "ivf_bulk_delete"],
        &[
            Value::String(istr("default")),
            node_ref_list(&[nodes[0], nodes[1]]),
        ],
    )
    .expect("IVF bulk delete succeeds");

    assert_eq!(ivf.snapshot().len(), 1);
}

#[test]
fn vector_ivf_bulk_upsert_rejects_wrong_dimension() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _, nodes) = graph_with_hnsw_and_ivf_nodes(87_908, &["Vec"]);

    let err = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "ivf_bulk_upsert"],
        &[
            Value::String(istr("default")),
            node_ref_list(&nodes),
            vector_matrix(&[&[1.0, 0.0]]),
        ],
    )
    .expect_err("wrong dimension rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_ivf_bulk_delete_requires_ivfp_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(87_909, &["Vec"]);

    let err = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "ivf_bulk_delete"],
        &[Value::String(istr("default")), node_ref_list(&[nodes[0]])],
    )
    .expect_err("missing IVFP provider rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}
