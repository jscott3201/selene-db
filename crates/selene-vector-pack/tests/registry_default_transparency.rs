//! Procedure-level default-index transparency tests for BRIEF-109 PR1.

use std::sync::Arc;

use selene_core::{Change, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ImplDefinedCaps, MutationContext, ProcedureContext,
    ProcedureError, ProcedureRegistry, ProcedureResult, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{HnswConfig, HnswIndexRegistry, HnswProvider};
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
fn default_path_via_registry_matches_singleton_for_upsert_search() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (singleton_graph, _singleton_provider, singleton_nodes) = singleton_graph(109_301);
    let (registry_graph, registry_provider, registry_nodes) = registry_graph(109_302);

    execute_mutation_direct(
        &singleton_graph,
        &procedures,
        &["vector", "upsert"],
        &upsert_args(singleton_nodes[0], vec![1.0, 0.0, 0.0, 0.0]),
    )
    .expect("singleton upsert succeeds");
    execute_mutation_direct(
        &registry_graph,
        &procedures,
        &["vector", "upsert"],
        &upsert_args(registry_nodes[0], vec![1.0, 0.0, 0.0, 0.0]),
    )
    .expect("registry upsert succeeds");

    let singleton_rows = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id, score",
        &singleton_graph,
        &procedures,
    ));
    let registry_rows = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id, score",
        &registry_graph,
        &procedures,
    ));

    assert_eq!(singleton_rows, registry_rows);
    assert_eq!(
        registry_provider
            .get("default")
            .expect("default provider exists")
            .snapshot()
            .len(),
        1
    );
}

#[test]
fn non_default_name_without_lifecycle_create_errors_cleanly() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (graph, _registry_provider, nodes) = registry_graph(109_303);

    let err = execute_mutation_direct(
        &graph,
        &procedures,
        &["vector", "upsert"],
        &[
            Value::String(istr("episodes")),
            Value::NodeRef(nodes[0]),
            vector_value(&[1.0, 0.0, 0.0, 0.0]),
        ],
    )
    .expect_err("unknown non-default index rejected");

    assert!(matches!(
        err,
        ProcedureError::InvalidArgument { detail }
            if detail.contains("VECT registry has no vector index 'episodes'")
    ));
}

#[test]
fn snapshot_round_trip_via_registry_uses_v1_wrapper_and_recovers_default() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (singleton_graph, singleton_provider, singleton_nodes) = singleton_graph(109_304);
    let (registry_graph, registry_provider, registry_nodes) = registry_graph(109_305);

    execute_mutation_direct(
        &singleton_graph,
        &procedures,
        &["vector", "upsert"],
        &upsert_args(singleton_nodes[0], vec![1.0, 0.0, 0.0, 0.0]),
    )
    .expect("singleton upsert succeeds");
    execute_mutation_direct(
        &registry_graph,
        &procedures,
        &["vector", "upsert"],
        &upsert_args(registry_nodes[0], vec![1.0, 0.0, 0.0, 0.0]),
    )
    .expect("registry upsert succeeds");

    let sections = hnsw_sections(registry_provider.as_ref());
    assert_v1_wrapped(&sections);

    let recovered = HnswIndexRegistry::new(hnsw_config()).expect("recovery registry builds");
    read_hnsw_sections(&recovered, &sections);

    assert_eq!(
        singleton_provider
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("singleton search succeeds"),
        recovered
            .get("default")
            .expect("default provider exists")
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("recovered search succeeds")
    );
}

#[test]
fn wal_payload_via_registry_byte_identical_to_singleton() {
    let pack = deterministic_pack();
    let procedures = registry(&pack);
    let (singleton_graph, _singleton_provider, singleton_nodes) = singleton_graph(109_306);
    let (registry_graph, _registry_provider, registry_nodes) = registry_graph(109_307);

    let singleton_changes = execute_mutation_batch_direct(
        &singleton_graph,
        &procedures,
        &[(
            &["vector", "upsert"],
            upsert_args(singleton_nodes[0], vec![1.0, 0.0, 0.0, 0.0]),
        )],
    )
    .expect("singleton mutation succeeds");
    let registry_changes = execute_mutation_batch_direct(
        &registry_graph,
        &procedures,
        &[(
            &["vector", "upsert"],
            upsert_args(registry_nodes[0], vec![1.0, 0.0, 0.0, 0.0]),
        )],
    )
    .expect("registry mutation succeeds");

    assert_eq!(
        extension_payloads(&singleton_changes, "selene-vector"),
        extension_payloads(&registry_changes, "selene-vector")
    );
}

fn deterministic_pack() -> VectorPack {
    VectorPack::with_config(VectorPackConfig {
        deterministic_seed: Some(0x1090_0001),
    })
}

fn singleton_graph(id: u64) -> (SharedGraph, Arc<HnswProvider>, Vec<NodeId>) {
    let provider = Arc::new(HnswProvider::new(hnsw_config()).expect("provider builds"));
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let node_ids = create_nodes(&graph);
    (graph, provider, node_ids)
}

fn registry_graph(id: u64) -> (SharedGraph, Arc<HnswIndexRegistry>, Vec<NodeId>) {
    let registry = Arc::new(HnswIndexRegistry::new(hnsw_config()).expect("registry builds"));
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(registry.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let node_ids = create_nodes(&graph);
    (graph, registry, node_ids)
}

fn create_nodes(graph: &SharedGraph) -> Vec<NodeId> {
    let mut txn = graph.begin_write();
    let node_id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(istr("Vec")), PropertyMap::new())
            .expect("fixture node inserts")
    };
    txn.commit().expect("fixture commit succeeds");
    vec![node_id]
}

fn hnsw_config() -> HnswConfig {
    HnswConfig::new(4).expect("HNSW config is valid")
}

fn upsert_args(node_id: NodeId, vector: Vec<f64>) -> Vec<Value> {
    vec![
        Value::String(istr("default")),
        Value::NodeRef(node_id),
        vector_value(&vector),
    ]
}

fn vector_value(vector: &[f64]) -> Value {
    Value::List(vector.iter().copied().map(Value::Float).collect())
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

fn assert_v1_wrapped(sections: &[Vec<u8>]) {
    for section in sections {
        assert!(section.starts_with(&[1, 0]));
    }
}

fn extension_payloads(changes: &[Change], provider_name: &str) -> Vec<Vec<u8>> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::IndexExtensionEvent { provider, payload }
                if provider.as_str() == provider_name =>
            {
                Some(payload.as_ref().to_vec())
            }
            _ => None,
        })
        .collect()
}
