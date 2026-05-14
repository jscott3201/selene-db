//! `vector.search` adapter tests.

use std::sync::Arc;

use selene_core::{Change, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ExecutorError, ImplDefinedCaps, MutationContext, ProcedureContext,
    ProcedureError, ProcedureRegistry, ProcedureResult, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_testing::VectorPackCorpus;
use selene_vector::{HnswConfig, HnswProvider, VectorOp, VectorUpsertPayloadV1};
use selene_vector_pack::{VECTOR_PROCEDURE_NAMES, VectorPack};

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

fn upsert_args(node_id: NodeId, vector: Vec<Value>) -> Vec<Value> {
    vec![
        Value::String(istr("default")),
        Value::NodeRef(node_id),
        Value::List(vector),
    ]
}

fn graph_with_vectors(id: u64) -> (SharedGraph, Arc<HnswProvider>, Vec<NodeId>) {
    graph_with_labeled_vectors(
        id,
        &[
            ("Allowed", vec![1.0, 0.0, 0.0, 0.0]),
            ("Allowed", vec![0.9, 0.1, 0.0, 0.0]),
            ("Other", vec![0.0, 1.0, 0.0, 0.0]),
            ("Other", vec![-1.0, 0.0, 0.0, 0.0]),
        ],
    )
}

fn graph_with_labeled_vectors(
    id: u64,
    rows: &[(&str, Vec<f32>)],
) -> (SharedGraph, Arc<HnswProvider>, Vec<NodeId>) {
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let dyn_provider: Arc<dyn IndexProvider> = provider.clone();
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(dyn_provider)
        .build()
        .expect("graph builds");
    let mut node_ids = Vec::with_capacity(rows.len());
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for (label, _) in rows {
            node_ids.push(
                mutator
                    .create_node(LabelSet::single(istr(label)), PropertyMap::new())
                    .expect("fixture node inserts"),
            );
        }
        txn.commit().expect("fixture commit succeeds");
    }
    for ((_, vector), node_id) in rows.iter().zip(node_ids.iter().copied()) {
        provider
            .on_change(&vector_change(VectorUpsertPayloadV1 {
                op: VectorOp::Insert,
                node_id,
                vector: vector.clone(),
                max_layer: 0,
            }))
            .expect("vector event applies");
    }
    (graph, provider, node_ids)
}

fn graph_with_nodes(id: u64, labels: &[&str]) -> (SharedGraph, Arc<HnswProvider>, Vec<NodeId>) {
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let dyn_provider: Arc<dyn IndexProvider> = provider.clone();
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(dyn_provider)
        .build()
        .expect("graph builds");
    let mut node_ids = Vec::with_capacity(labels.len());
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for label in labels {
            node_ids.push(
                mutator
                    .create_node(LabelSet::single(istr(label)), PropertyMap::new())
                    .expect("fixture node inserts"),
            );
        }
        txn.commit().expect("fixture commit succeeds");
    }
    (graph, provider, node_ids)
}

fn vector_change(payload: VectorUpsertPayloadV1) -> Change {
    Change::IndexExtensionEvent {
        provider: istr("selene-vector"),
        payload: Arc::from(payload.encode().unwrap().into_boxed_slice()),
    }
}

fn node_ids(table: &BindingTable) -> Vec<NodeId> {
    node_ids_for(table, "node_id")
}

fn node_ids_for(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .schema()
        .columns
        .iter()
        .position(|column| column.name.is_some_and(|column| column.as_str() == name))
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

#[test]
fn vector_search_registers_metadata_and_capability_free_name() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let metadata = VECTOR_PROCEDURE_NAMES
        .iter()
        .map(|name| {
            let interned: Vec<_> = name.iter().map(|segment| istr(segment)).collect();
            registry.lookup(&interned).expect("procedure is registered")
        })
        .collect::<Vec<_>>();

    assert_eq!(metadata[0].signature.parameters.len(), 5);
    assert_eq!(metadata[0].output_schema.columns.len(), 2);
    assert_eq!(metadata[1].signature.parameters.len(), 3);
    assert_eq!(metadata[1].output_schema.columns.len(), 0);
    assert_eq!(metadata[2].signature.parameters.len(), 2);
    assert_eq!(metadata[2].output_schema.columns.len(), 0);
    assert!(
        metadata
            .iter()
            .all(|metadata| metadata.capability_required.is_none())
    );
}

#[test]
fn vector_search_returns_rows_matching_direct_provider_default_search() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, provider, _) = graph_with_vectors(8_701);

    let table = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 3, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected: Vec<_> = provider
        .search(&[1.0, 0.0, 0.0, 0.0], 3, None, None)
        .expect("direct search succeeds")
        .into_iter()
        .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(f64::from(score))])
        .collect();

    assert_eq!(
        table
            .rows()
            .iter()
            .map(|row| row.values().to_vec())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn vector_search_passes_zero_ef_search_through_to_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, provider, _) = graph_with_vectors(8_702);

    let table = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 2, 0, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected: Vec<_> = provider
        .search(&[1.0, 0.0, 0.0, 0.0], 2, Some(0), None)
        .expect("direct search succeeds")
        .into_iter()
        .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(f64::from(score))])
        .collect();

    assert_eq!(
        table
            .rows()
            .iter()
            .map(|row| row.values().to_vec())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn vector_search_accepts_literal_node_ref_filter_list() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_labeled_vectors(
        8_703,
        &[
            ("A", vec![1.0, 0.0, 0.0, 0.0]),
            ("B", vec![0.9, 0.1, 0.0, 0.0]),
            ("Other", vec![0.0, 1.0, 0.0, 0.0]),
        ],
    );

    let table = rows(execute_ok(
        "MATCH (a:A), (b:B) \
         CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 10, NULL, [a, b]) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(node_ids(&table), vec![nodes[0], nodes[1]]);
}

#[test]
fn vector_search_accepts_dynamic_node_ref_filter_binding() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_vectors(8_704);

    let table = rows(execute_ok(
        "MATCH (n:Allowed) WITH collect(n) AS nodes \
         CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 10, NULL, nodes) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(node_ids(&table), vec![nodes[0], nodes[1]]);
}

#[test]
fn vector_search_rejects_unknown_index_name_sentinel() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = graph_with_vectors(8_705);

    let err = execute_result(
        "CALL vector.search('embedding_idx', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("unknown index is rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn vector_search_dimension_mismatch_maps_to_invalid_argument() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, _) = graph_with_vectors(8_706);

    let err = execute_result(
        "CALL vector.search('default', [1.0, 0.0], 1, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("dimension mismatch is rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn vector_search_requires_registered_vect_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = SharedGraph::new(GraphId::new(8_707));

    let err = execute_result(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("missing provider is rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn vector_upsert_inserts_vector_and_search_returns_it() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _provider, nodes) = graph_with_nodes(8_709, &["Vec"]);

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "upsert"],
        &upsert_args(
            nodes[0],
            vec![
                Value::Float(1.0),
                Value::Float(0.0),
                Value::Float(0.0),
                Value::Float(0.0),
            ],
        ),
    )
    .expect("upsert succeeds");
    let table = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(node_ids(&table), vec![nodes[0]]);
}

#[test]
fn vector_upsert_with_wrong_dim_errors_at_adapter() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(8_710, &["Vec"]);

    let err = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "upsert"],
        &upsert_args(nodes[0], vec![Value::Float(1.0), Value::Float(0.0)]),
    )
    .expect_err("dimension mismatch is rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_upsert_with_nan_component_errors_at_adapter() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(8_711, &["Vec"]);

    let err = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "upsert"],
        &upsert_args(
            nodes[0],
            vec![
                Value::Float(f64::NAN),
                Value::Float(0.0),
                Value::Float(0.0),
                Value::Float(0.0),
            ],
        ),
    )
    .expect_err("non-finite component is rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_upsert_duplicate_node_id_returns_invalid_argument() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(8_712, &["Vec"]);
    let args = upsert_args(
        nodes[0],
        vec![
            Value::Float(1.0),
            Value::Float(0.0),
            Value::Float(0.0),
            Value::Float(0.0),
        ],
    );
    execute_mutation_direct(&graph, &registry, &["vector", "upsert"], &args)
        .expect("first upsert succeeds");

    let err = execute_mutation_direct(&graph, &registry, &["vector", "upsert"], &args)
        .expect_err("duplicate is rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_delete_removes_vector_from_search() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(8_713, &["Vec"]);
    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "upsert"],
        &upsert_args(
            nodes[0],
            vec![
                Value::Float(1.0),
                Value::Float(0.0),
                Value::Float(0.0),
                Value::Float(0.0),
            ],
        ),
    )
    .unwrap();

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "delete"],
        &[Value::String(istr("default")), Value::NodeRef(nodes[0])],
    )
    .expect("delete succeeds");
    let table = rows(execute_ok(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert!(table.rows().is_empty());
}

#[test]
fn vector_delete_of_non_existent_node_succeeds_silently() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(8_714, &["Vec"]);

    execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "delete"],
        &[Value::String(istr("default")), Value::NodeRef(nodes[0])],
    )
    .expect("missing delete succeeds");
}

#[test]
fn vector_delete_with_unknown_index_name_errors() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let (graph, _, nodes) = graph_with_nodes(8_715, &["Vec"]);

    let err = execute_mutation_direct(
        &graph,
        &registry,
        &["vector", "delete"],
        &[
            Value::String(istr("embedding_idx")),
            Value::NodeRef(nodes[0]),
        ],
    )
    .expect_err("unknown index rejected");

    assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
}

#[test]
fn vector_search_rejects_non_hnsw_vect_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = SharedGraph::builder(GraphId::new(8_708))
        .with_provider(Arc::new(WrongVectProvider) as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");

    let err = execute_result(
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("wrong provider type is rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn vector_pack_corpus_renders_in_deterministic_order_and_covers_registry() {
    let corpus = VectorPackCorpus::b2_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<Vec<_>>();

    assert_eq!(observed, VECTOR_PROCEDURE_NAMES.to_vec());
    insta::assert_snapshot!(corpus.render(), @r"
search_default [Search] CALL vector.search('default', [1.000000, 0.000000, 0.000000, 0.000000], 10, NULL, NULL)
upsert_default [Upsert] CALL vector.upsert('default', 42, [1.000000, 0.000000, 0.000000, 0.000000])
delete_default [Delete] CALL vector.delete('default', 42)
");
}

struct WrongVectProvider;

impl IndexProvider for WrongVectProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"VECT")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}
