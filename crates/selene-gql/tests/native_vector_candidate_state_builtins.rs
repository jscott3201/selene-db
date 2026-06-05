//! End-to-end coverage for maintained candidate-state vector built-ins.

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

fn candidate_graph() -> (SharedGraph, Vec<NodeId>) {
    let state_name = istr("active_docs");
    let doc = istr("VectorDoc");
    let superseded = istr("SUPERSEDED_BY");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([CandidateStateSpec::new(state_name)
            .require_label(doc.clone())
            .exclude_outgoing(superseded.clone())])
        .expect("provider config is valid"),
    );
    let graph = SharedGraph::builder(GraphId::new(330_401))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let embedding = istr("embedding");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts"),
        );
    }
    mutator
        .create_edge(superseded, ids[4], ids[0], PropertyMap::new())
        .expect("stale edge inserts");
    txn.commit().expect("seed commits");
    (graph, ids)
}

#[test]
fn vector_score_candidate_state_reranks_maintained_set() {
    let (graph, ids) = candidate_graph();
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[2.2, 0.0])));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_candidate_state('embedding', $query, 'active_docs', 3) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![ids[2], ids[3], ids[1]]);
}

#[test]
fn vector_score_candidate_state_rejects_unknown_set() {
    let (graph, _) = candidate_graph();
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let err = session
        .execute_source(
            "CALL selene.vector_score_candidate_state('embedding', $query, 'missing', 3)",
            &registry,
        )
        .expect_err("unknown state must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("unknown maintained candidate-state set 'missing'")
    ));
}

#[test]
fn vector_score_candidate_state_surfaces_stale_provider_generation() {
    let provider = Arc::new(StaleCandidateProvider);
    let graph = SharedGraph::builder(GraphId::new(330_402))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let err = session
        .execute_source(
            "CALL selene.vector_score_candidate_state('embedding', $query, 'active_docs', 3)",
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

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}
