//! Deterministic HNSW layer assignment tests for `vector.upsert` and
//! `vector.bulk_upsert`.

use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    ImplDefinedCaps, MutationContext, ProcedureContext, ProcedureError, ProcedureRegistry,
    ProcedureResult,
};
use selene_graph::{IndexProvider, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{HnswConfig, HnswProvider};
use selene_vector_pack::{VectorPack, VectorPackConfig};

const ROW_COUNT: usize = 48;
const DETERMINISTIC_SEED: u64 = 42;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TopologyRow {
    node_id: u64,
    max_layer: u8,
    neighbors: Vec<Vec<u32>>,
}

#[derive(Clone, Copy)]
enum BuildMode {
    Upsert,
    BulkUpsert,
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn registry(pack: &VectorPack) -> ProcedurePackRegistry {
    pack.registry_with_builtins()
        .expect("vector pack registers cleanly")
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

fn graph_with_nodes(id: u64, count: usize) -> (SharedGraph, Arc<HnswProvider>, Vec<NodeId>) {
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let mut node_ids = Vec::with_capacity(count);
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        for _ in 0..count {
            node_ids.push(
                mutator
                    .create_node(LabelSet::single(istr("Vec")), PropertyMap::new())
                    .expect("fixture node inserts"),
            );
        }
    }
    txn.commit().expect("fixture commit succeeds");
    (graph, provider, node_ids)
}

fn vector_for(row: usize) -> Vec<f64> {
    let raw = row as f64 + 1.0;
    vec![
        raw / 97.0,
        ((row * 7 + 3) % 23) as f64 / 23.0,
        ((row * 11 + 5) % 29) as f64 / 29.0,
        ((row * 13 + 7) % 31) as f64 / 31.0,
    ]
}

fn vector_value(vector: &[f64]) -> Value {
    Value::List(vector.iter().copied().map(Value::Float).collect())
}

fn node_ref_list(node_ids: &[NodeId]) -> Value {
    Value::List(node_ids.iter().copied().map(Value::NodeRef).collect())
}

fn vector_matrix(vectors: &[Vec<f64>]) -> Value {
    Value::List(vectors.iter().map(|row| vector_value(row)).collect())
}

fn build_index(id: u64, mode: BuildMode) -> Arc<HnswProvider> {
    let pack = VectorPack::with_config(VectorPackConfig {
        deterministic_seed: Some(DETERMINISTIC_SEED),
    });
    let registry = registry(&pack);
    let (graph, provider, nodes) = graph_with_nodes(id, ROW_COUNT);
    let vectors = (0..ROW_COUNT).map(vector_for).collect::<Vec<_>>();

    match mode {
        BuildMode::Upsert => {
            for (node_id, vector) in nodes.iter().copied().zip(&vectors) {
                execute_mutation_direct(
                    &graph,
                    &registry,
                    &["vector", "upsert"],
                    &[
                        Value::String(istr("default")),
                        Value::NodeRef(node_id),
                        vector_value(vector),
                    ],
                )
                .expect("upsert succeeds");
            }
        }
        BuildMode::BulkUpsert => {
            execute_mutation_direct(
                &graph,
                &registry,
                &["vector", "bulk_upsert"],
                &[
                    Value::String(istr("default")),
                    node_ref_list(&nodes),
                    vector_matrix(&vectors),
                ],
            )
            .expect("bulk upsert succeeds");
        }
    }

    provider
}

fn topology(provider: &HnswProvider) -> Vec<TopologyRow> {
    provider
        .snapshot()
        .iter_nodes()
        .map(|node| TopologyRow {
            node_id: node.node_id.get(),
            max_layer: node.max_layer,
            neighbors: node.neighbors.iter().cloned().collect(),
        })
        .collect()
}

fn snapshot_bytes(provider: &HnswProvider) -> (Vec<u8>, Vec<u8>) {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    (grph, vecs)
}

fn assert_same_graph(left: &HnswProvider, right: &HnswProvider) {
    assert_eq!(topology(left), topology(right));
    assert_eq!(snapshot_bytes(left), snapshot_bytes(right));
}

#[test]
fn deterministic_seed_makes_upsert_snapshot_bytes_reproducible() {
    let first = build_index(95_201, BuildMode::Upsert);
    let second = build_index(95_202, BuildMode::Upsert);

    assert_same_graph(&first, &second);
}

#[test]
fn deterministic_seed_makes_bulk_upsert_snapshot_bytes_reproducible() {
    let first = build_index(95_203, BuildMode::BulkUpsert);
    let second = build_index(95_204, BuildMode::BulkUpsert);

    assert_same_graph(&first, &second);
}

#[test]
fn deterministic_seed_aligns_upsert_and_bulk_upsert_topology() {
    let upsert = build_index(95_205, BuildMode::Upsert);
    let bulk = build_index(95_206, BuildMode::BulkUpsert);

    assert_same_graph(&upsert, &bulk);
}
