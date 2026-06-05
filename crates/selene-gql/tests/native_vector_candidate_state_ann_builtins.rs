//! End-to-end coverage for ANN-expanded maintained candidate-state vector built-ins.

use std::sync::Arc;

use selene_core::{
    Change, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern,
};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::{
    CANDIDATE_STATE_PROVIDER_TAG, CandidateStateSpec, IndexProvider,
    MaintainedCandidateStateProvider, ProviderError, ProviderTag, SharedGraph, SubTag,
    VectorCandidateStateInfo,
};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
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

fn ann_state_graph(id: u64) -> (SharedGraph, AnnStateIds) {
    let active_fact = istr("ActiveFact");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([
            CandidateStateSpec::new(istr("active_facts")).require_label(active_fact.clone())
        ])
        .expect("candidate-state provider config is valid"),
    );
    let graph = SharedGraph::builder(GraphId::new(id))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let summary = istr("Summary");
    let fact = istr("Fact");
    let embedding = istr("embedding");
    let supports = istr("SUPPORTS");

    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let root_a = mutator
        .create_node(
            LabelSet::single(summary.clone()),
            props(&embedding, Value::Vector(vector(&[0.2, 0.0]))),
        )
        .expect("root a inserts");
    let fact_a = mutator
        .create_node(
            LabelSet::from_iter([fact.clone(), active_fact.clone()]),
            props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
        )
        .expect("fact a inserts");
    let stale_fact = mutator
        .create_node(
            LabelSet::single(fact.clone()),
            props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
        )
        .expect("stale fact inserts");
    let root_b = mutator
        .create_node(
            LabelSet::single(summary),
            props(&embedding, Value::Vector(vector(&[10.2, 0.0]))),
        )
        .expect("root b inserts");
    let fact_b = mutator
        .create_node(
            LabelSet::from_iter([fact, active_fact]),
            props(&embedding, Value::Vector(vector(&[10.0, 0.0]))),
        )
        .expect("fact b inserts");
    mutator
        .create_edge(supports.clone(), root_a, fact_a, PropertyMap::new())
        .expect("support edge a inserts");
    mutator
        .create_edge(supports.clone(), root_a, stale_fact, PropertyMap::new())
        .expect("stale support edge inserts");
    mutator
        .create_edge(supports, root_b, fact_b, PropertyMap::new())
        .expect("support edge b inserts");
    txn.commit().expect("seed commits");

    (
        graph,
        AnnStateIds {
            root_a,
            fact_a,
            stale_fact,
            root_b,
            fact_b,
        },
    )
}

fn create_hnsw_index(
    session: &mut Session<'_>,
    registry: &BuiltinProcedureRegistry,
    dimension: usize,
) {
    session
        .execute_source(
            &format!(
                "CALL selene.create_vector_index('Summary', 'embedding', {dimension}, 'hnsw')"
            ),
            registry,
        )
        .expect("hnsw vector index creation executes");
}

#[derive(Clone, Copy)]
struct AnnStateIds {
    root_a: NodeId,
    fact_a: NodeId,
    stale_fact: NodeId,
    root_b: NodeId,
    fact_b: NodeId,
}

#[test]
fn vector_search_candidate_state_expanded_ann_intersects_state_with_expanded_roots() {
    let (graph, ids) = ann_state_graph(330_511);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    create_hnsw_index(&mut session, &registry, 2);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_candidate_state_expanded_ann(
            'Summary', 'embedding', $query, 'active_facts', 1, 'SUPPORTS', 3
         ) YIELD node_id, distance",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![ids.fact_a]);
}

#[test]
fn vector_search_candidate_state_expanded_ann_supports_union_algebra() {
    let (graph, ids) = ann_state_graph(330_512);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    create_hnsw_index(&mut session, &registry, 2);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_candidate_state_expanded_ann(
            'Summary', 'embedding', $query, 'active_facts', 1, 'SUPPORTS', 5,
            'union', 'outgoing', 'squared_euclidean', 32
         ) YIELD node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![ids.fact_a, ids.stale_fact, ids.root_a, ids.fact_b]
    );
    assert!(!node_column(&table, "node_id").contains(&ids.root_b));
}

#[test]
fn vector_search_candidate_state_expanded_ann_rejects_unknown_operation() {
    let (graph, _ids) = ann_state_graph(330_513);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    create_hnsw_index(&mut session, &registry, 2);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let err = session
        .execute_source(
            "CALL selene.vector_search_candidate_state_expanded_ann(
                'Summary', 'embedding', $query, 'active_facts', 1, 'SUPPORTS', 3,
                'xor'
             )",
            &registry,
        )
        .expect_err("unknown operation must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("operation must be intersection")
    ));
}

#[test]
fn vector_search_candidate_state_expanded_ann_requires_ann_index() {
    let (graph, _ids) = ann_state_graph(330_514);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let err = session
        .execute_source(
            "CALL selene.vector_search_candidate_state_expanded_ann(
                'Summary', 'embedding', $query, 'active_facts', 1, 'SUPPORTS', 3
             )",
            &registry,
        )
        .expect_err("missing ANN index must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("requires a matching ANN vector index")
    ));
}

#[test]
fn vector_search_candidate_state_expanded_ann_surfaces_stale_provider_generation() {
    let provider = Arc::new(StaleCandidateProvider);
    let graph = SharedGraph::builder(GraphId::new(330_515))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let err = session
        .execute_source(
            "CALL selene.vector_search_candidate_state_expanded_ann(
                'Summary', 'embedding', $query, 'active_facts', 1, 'SUPPORTS', 3
             )",
            &registry,
        )
        .expect_err("stale provider must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { ref detail },
            ..
        } if detail.contains("candidate-state provider error")
            && detail.contains("stale candidate state")
    ));
}

struct StaleCandidateProvider;

impl IndexProvider for StaleCandidateProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(CANDIDATE_STATE_PROVIDER_TAG)
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

    fn vector_candidate_set(
        &self,
        _name: &IStr,
        _generation: u64,
    ) -> Result<Option<selene_graph::VectorCandidateSet>, ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: "stale candidate state".to_owned(),
        })
    }

    fn vector_candidate_state_infos(
        &self,
        _generation: u64,
    ) -> Result<Vec<VectorCandidateStateInfo>, ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: "stale candidate state".to_owned(),
        })
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}
