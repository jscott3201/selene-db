//! BRIEF-109 PR2 named-index lifecycle tests.

use std::sync::Arc;

use selene_core::{Change, GraphId, IStr, LabelSet, NodeId, PropertyMap, Record, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ImplDefinedCaps, MutationContext, ProcedureContext,
    ProcedureError, ProcedureRegistry, ProcedureResult, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{
    Catalog, DistanceMetric, HnswConfig, HnswIndexRegistry, IvfConfig, IvfIndexRegistry, PqParams,
    VectorIvfBulkInsertV1, encode_named_payload,
};
use selene_vector_pack::{VectorPack, VectorPackConfig};

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

#[test]
fn hnsw_named_lifecycle_wal_replay_and_snapshot_recover() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (graph, hnsw, _ivf, nodes) = graph_with_registries(109_401, 3);

    let changes = execute_mutation_batch_direct(
        &graph,
        &procedures,
        &[
            (
                &["vector", "create_index"],
                create_index_args("episodes", "hnsw", &[("dim", Value::Int(2))]),
            ),
            (
                &["vector", "upsert"],
                vec![
                    Value::String(istr("episodes")),
                    Value::NodeRef(nodes[0]),
                    vector(&[1.0, 0.0]),
                ],
            ),
        ],
    )
    .expect("create + upsert succeeds");

    let replay_hnsw = HnswIndexRegistry::new(hnsw_config()).expect("HNSW replay registry builds");
    let replay_ivf = IvfIndexRegistry::new(ivf_config()).expect("IVF replay registry builds");
    for change in &changes {
        replay_hnsw.on_change(change).expect("HNSW replay applies");
        replay_ivf.on_change(change).expect("IVF replay applies");
    }
    assert_eq!(search_node(&replay_hnsw, "episodes", &[1.0, 0.0]), nodes[0]);

    let sections = hnsw_sections(hnsw.as_ref());
    assert_v1_wrapped(&sections);
    let recovered = HnswIndexRegistry::new(hnsw_config()).expect("HNSW recovery registry builds");
    read_hnsw_sections(&recovered, &sections);
    assert_eq!(search_node(&recovered, "episodes", &[1.0, 0.0]), nodes[0]);
}

#[test]
fn create_index_idempotent_and_conflicting_config_errors() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (graph, _hnsw, _ivf, _nodes) = graph_with_registries(109_402, 1);

    execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args("episodes", "hnsw", &[("dim", Value::Int(2))]),
    )
    .expect("initial create succeeds");
    execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args("episodes", "hnsw", &[("dim", Value::Int(2))]),
    )
    .expect("same-config create is idempotent");

    let err = execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args("episodes", "hnsw", &[("dim", Value::Int(3))]),
    )
    .expect_err("conflicting create rejected");
    assert!(matches!(
        err,
        ProcedureError::InvalidArgument { detail }
            if detail.contains("different HNSW config")
    ));
}

#[test]
fn create_index_rejects_oversized_name() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (graph, hnsw, ivf, _nodes) = graph_with_registries(109_405, 1);
    let oversized = "x".repeat(u16::MAX as usize + 1);

    let err = execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args(&oversized, "hnsw", &[("dim", Value::Int(2))]),
    )
    .expect_err("oversized create rejected");
    assert!(matches!(
        err,
        ProcedureError::InvalidArgument { detail } if detail.contains("wire limit")
    ));

    let catalog = Catalog::from_registries(hnsw, ivf);
    assert!(
        catalog
            .list_indexes()
            .iter()
            .all(|entry| entry.name.as_ref() != oversized)
    );
}

#[test]
fn drop_default_rejected_and_list_indexes_reports_both_kinds() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (graph, _hnsw, _ivf, _nodes) = graph_with_registries(109_403, 1);

    let err = execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "drop_index"],
        &[Value::String(istr("default"))],
    )
    .expect_err("default drop rejected");
    assert!(matches!(
        err,
        ProcedureError::InvalidArgument { detail }
            if detail.contains("cannot drop the default vector index")
    ));

    execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args("episodes", "hnsw", &[("dim", Value::Int(2))]),
    )
    .expect("HNSW create succeeds");
    execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args("staged", "ivf", &[("dim", Value::Int(2))]),
    )
    .expect("IVF create succeeds");

    let table = rows(execute_ok(
        "CALL vector.list_indexes() YIELD name, kind, dim, metric, vector_count",
        &graph,
        &procedures,
    ));
    let rendered = table
        .rows()
        .iter()
        .map(|row| {
            row.values()
                .iter()
                .map(render_value)
                .collect::<Vec<_>>()
                .join(":")
        })
        .collect::<Vec<_>>();
    assert!(
        rendered
            .iter()
            .any(|row| row.starts_with("episodes:hnsw:2"))
    );
    assert!(rendered.iter().any(|row| row.starts_with("staged:ivf:2")));
    assert!(rendered.iter().any(|row| row.starts_with("default:hnsw:4")));
    assert!(rendered.iter().any(|row| row.starts_with("default:ivf:4")));
}

#[test]
fn ivf_deferred_state_survives_named_snapshot_recovery() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (graph, _hnsw, ivf, nodes) = graph_with_registries(109_404, 260);

    execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "create_index"],
        &create_index_args("staged", "ivf", &[("dim", Value::Int(2))]),
    )
    .expect("IVF create succeeds");
    execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "ivf_bulk_upsert"],
        &[
            Value::String(istr("staged")),
            node_ref_list(&nodes[0..2]),
            vector_matrix(&[[1.0, 0.0], [0.0, 1.0]]),
        ],
    )
    .expect("below-threshold IVF upsert succeeds");

    let sections = ivf_sections(ivf.as_ref());
    assert_v1_wrapped(&sections);
    let recovered = IvfIndexRegistry::new(ivf_config()).expect("IVF recovery registry builds");
    read_ivf_sections(&recovered, &sections);
    assert_eq!(
        recovered
            .get("staged")
            .expect("named IVF exists")
            .snapshot()
            .len(),
        2
    );

    let rows = (2..256)
        .map(|idx| [idx as f64, (idx % 7) as f64])
        .collect::<Vec<_>>();
    let payload = VectorIvfBulkInsertV1 {
        rows: nodes[2..256]
            .iter()
            .copied()
            .zip(rows.iter())
            .map(|(node_id, vector)| selene_vector::IvfBulkInsertRow {
                node_id,
                vector: vector.iter().map(|value| *value as f32).collect(),
            })
            .collect(),
    };
    let bytes = encode_named_payload("staged", payload.encode().unwrap()).unwrap();
    let change = Change::IndexExtensionEvent {
        provider: istr("selene-vector-ivf"),
        payload: Arc::from(bytes.into_boxed_slice()),
    };
    recovered
        .on_change(&change)
        .expect("post-recovery named IVF upsert applies");
    let _ = ivf_sections(&recovered);
    assert!(
        recovered
            .get("staged")
            .expect("named IVF exists")
            .snapshot()
            .is_trained()
    );
}

fn deterministic_pack() -> VectorPack {
    VectorPack::with_config(VectorPackConfig {
        deterministic_seed: Some(0x1090_0002),
    })
}

fn graph_with_registries(
    id: u64,
    node_count: usize,
) -> (
    SharedGraph,
    Arc<HnswIndexRegistry>,
    Arc<IvfIndexRegistry>,
    Vec<NodeId>,
) {
    let hnsw = Arc::new(HnswIndexRegistry::new(hnsw_config()).expect("HNSW registry builds"));
    let ivf = Arc::new(IvfIndexRegistry::new(ivf_config()).expect("IVF registry builds"));
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(hnsw.clone() as Arc<dyn IndexProvider>)
        .with_provider(ivf.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let nodes = create_nodes(&graph, node_count);
    (graph, hnsw, ivf, nodes)
}

fn create_nodes(graph: &SharedGraph, count: usize) -> Vec<NodeId> {
    let mut txn = graph.begin_write();
    let mut ids = Vec::with_capacity(count);
    {
        let mut mutator = txn.mutator();
        for _ in 0..count {
            ids.push(
                mutator
                    .create_node(LabelSet::single(istr("Vec")), PropertyMap::new())
                    .expect("fixture node inserts"),
            );
        }
    }
    txn.commit().expect("fixture commit succeeds");
    ids
}

fn hnsw_config() -> HnswConfig {
    HnswConfig::new(4).expect("HNSW config is valid")
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
    .expect("IVF config is valid")
}

fn create_index_args(name: &str, kind: &str, fields: &[(&str, Value)]) -> Vec<Value> {
    vec![
        Value::String(istr(name)),
        Value::String(istr(kind)),
        record(fields),
    ]
}

fn record(fields: &[(&str, Value)]) -> Value {
    Value::Record(Box::new(Record::Open(
        fields
            .iter()
            .map(|(name, value)| (istr(name), value.clone()))
            .collect(),
    )))
}

fn vector(values: &[f64]) -> Value {
    Value::List(values.iter().copied().map(Value::Float).collect())
}

fn node_ref_list(nodes: &[NodeId]) -> Value {
    Value::List(nodes.iter().copied().map(Value::NodeRef).collect())
}

fn vector_matrix(rows: &[[f64; 2]]) -> Value {
    Value::List(rows.iter().map(|row| vector(row)).collect())
}

fn hnsw_sections(provider: &dyn IndexProvider) -> Vec<Vec<u8>> {
    [SubTag(*b"GRPH"), SubTag(*b"VECS"), SubTag(*b"QUNT")]
        .into_iter()
        .map(|sub_tag| {
            provider
                .write_section(sub_tag)
                .expect("HNSW section writes")
        })
        .collect()
}

fn read_hnsw_sections(provider: &dyn IndexProvider, sections: &[Vec<u8>]) {
    for (sub_tag, bytes) in [SubTag(*b"GRPH"), SubTag(*b"VECS"), SubTag(*b"QUNT")]
        .into_iter()
        .zip(sections)
    {
        provider
            .read_section(sub_tag, bytes)
            .expect("HNSW section reads");
    }
}

fn ivf_sections(provider: &dyn IndexProvider) -> Vec<Vec<u8>> {
    [SubTag(*b"CQNT"), SubTag(*b"IPQB"), SubTag(*b"POST")]
        .into_iter()
        .map(|sub_tag| provider.write_section(sub_tag).expect("IVF section writes"))
        .collect()
}

fn read_ivf_sections(provider: &dyn IndexProvider, sections: &[Vec<u8>]) {
    for (sub_tag, bytes) in [SubTag(*b"CQNT"), SubTag(*b"IPQB"), SubTag(*b"POST")]
        .into_iter()
        .zip(sections)
    {
        provider
            .read_section(sub_tag, bytes)
            .expect("IVF section reads");
    }
}

fn assert_v1_wrapped(sections: &[Vec<u8>]) {
    for section in sections {
        assert!(section.starts_with(&[1, 0]));
    }
}

fn search_node(registry: &HnswIndexRegistry, index: &str, query: &[f32]) -> NodeId {
    registry
        .get(index)
        .expect("index exists")
        .search(query, 1, None, None, None)
        .expect("search succeeds")
        .first()
        .map(|(node_id, _)| *node_id)
        .expect("one result")
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.as_str().to_owned(),
        Value::ExternalString(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        other => format!("{other:?}"),
    }
}
