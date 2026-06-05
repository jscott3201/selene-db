use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
};
use selene_testing::local_omlx::{CorpusInput, Topic, topic_label};

const QUERY_ROOT_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_expanded_candidates('embedding', $query, roots, 'OmlxSupports', 4, 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_STATE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_candidate_state_expanded('embedding', $query, 'omlx_support_facts', roots, 'OmlxSupports', 4, 'intersection', 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_CURRENT_STATE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_candidate_state_expanded('embedding', $query, 'omlx_current_support_facts', roots, 'OmlxSupports', 4, 'intersection', 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, anchor.query AS query, collect_list(root) AS roots GROUP BY anchor.query_index, anchor.query ORDER BY query_index WITH collect_list(query) AS queries, collect_list(roots) AS root_sets CALL selene.vector_score_expanded_candidates_batch('embedding', queries, root_sets, 'OmlxSupports', 4, 'outgoing', 'cosine') YIELD query_index, node_id, distance RETURN query_index, node_id, distance";

pub(super) const TOP_K: usize = 4;

pub(super) struct OmlxGqlQueryRootFixture {
    graph: SharedGraph,
    pub(super) dimension: usize,
    documents: Vec<DocumentMeta>,
    topics_by_node: HashMap<NodeId, Topic>,
    current_by_node: HashMap<NodeId, bool>,
    queries: Vec<QueryVector>,
}

impl OmlxGqlQueryRootFixture {
    pub(super) fn build(
        model: &str,
        inputs: &[CorpusInput],
        vectors: Vec<VectorValue>,
        graph_hint_docs_per_topic: Option<usize>,
    ) -> Self {
        assert_eq!(
            vectors.len(),
            inputs.len(),
            "oMLX returned one vector per corpus input"
        );
        let dimension = vectors
            .first()
            .map(VectorValue::dimension)
            .expect("corpus has at least one vector");
        assert!(
            vectors.iter().all(|vector| vector.dimension() == dimension),
            "oMLX returned consistent vector dimensions"
        );
        let doc_label = istr("OmlxEmbeddingDoc");
        let query_label = istr("OmlxQueryAnchor");
        let support_fact_label = istr("OmlxSupportFact");
        let dependency_edge = istr("OmlxDependsOn");
        let support_edge = istr("OmlxSupports");
        let negative_evidence_edge = istr("OmlxNegativeEvidence");
        let embedding_key = istr("embedding");
        let query_key = istr("query");
        let query_index_key = istr("query_index");
        let support_state_provider = Arc::new(
            MaintainedCandidateStateProvider::new([
                CandidateStateSpec::new(istr("omlx_support_facts"))
                    .require_label(support_fact_label.clone()),
                CandidateStateSpec::new(istr("omlx_current_support_facts"))
                    .require_label(support_fact_label.clone())
                    .exclude_outgoing(negative_evidence_edge.clone()),
            ])
            .expect("oMLX GQL maintained support state provider is valid"),
        );
        let graph = SharedGraph::builder(graph_id_for_model(model))
            .with_provider(support_state_provider as Arc<dyn IndexProvider>)
            .build()
            .expect("oMLX GQL bench graph builds");
        let mut documents = Vec::new();
        let mut query_anchors = Vec::new();
        let mut graph_hint_counts = HashMap::<Topic, usize>::new();
        {
            let mut txn = graph.begin_write();
            {
                let mut mutator = txn.mutator();
                for (input, vector) in inputs.iter().zip(vectors.iter()) {
                    if !input.is_document {
                        continue;
                    }
                    let props = PropertyMap::from_pairs([(
                        embedding_key.clone(),
                        Value::Vector(vector.clone()),
                    )])
                    .expect("oMLX GQL bench document properties fit");
                    let graph_hint = admits_graph_hint(
                        &mut graph_hint_counts,
                        input.topic,
                        graph_hint_docs_per_topic,
                    );
                    let mut labels = LabelSet::single(doc_label.clone());
                    if graph_hint {
                        labels.insert(topic_label(input.topic));
                    }
                    let support_fact = !graph_hint || graph_hint_docs_per_topic.is_none();
                    if support_fact {
                        labels.insert(support_fact_label.clone());
                    }
                    let current_fact = !is_negative_evidence_document(input.text);
                    let node = mutator
                        .create_node(labels, props)
                        .expect("oMLX GQL bench document node inserts");
                    documents.push(DocumentMeta {
                        node,
                        topic: input.topic,
                        graph_hint,
                        support_fact,
                        current_fact,
                    });
                }
                for source in documents.iter().filter(|document| document.graph_hint) {
                    for target in documents
                        .iter()
                        .filter(|document| document.topic == source.topic)
                    {
                        if target.node == source.node {
                            continue;
                        }
                        mutator
                            .create_edge(
                                support_edge.clone(),
                                source.node,
                                target.node,
                                PropertyMap::new(),
                            )
                            .expect("oMLX GQL bench support edge inserts");
                    }
                }
                for stale in documents.iter().filter(|document| !document.current_fact) {
                    let replacement = documents
                        .iter()
                        .find(|document| document.topic == stale.topic && document.current_fact)
                        .expect("each oMLX topic has a current replacement fact");
                    mutator
                        .create_edge(
                            negative_evidence_edge.clone(),
                            stale.node,
                            replacement.node,
                            PropertyMap::new(),
                        )
                        .expect("oMLX GQL bench negative-evidence edge inserts");
                }
                for (query_index, (input, vector)) in inputs
                    .iter()
                    .zip(vectors.iter())
                    .filter(|(input, _)| !input.is_document)
                    .enumerate()
                {
                    let props = PropertyMap::from_pairs([
                        (query_index_key.clone(), Value::Int(query_index as i64)),
                        (query_key.clone(), Value::Vector(vector.clone())),
                    ])
                    .expect("oMLX GQL bench query properties fit");
                    let anchor = mutator
                        .create_node(LabelSet::from_iter([query_label.clone()]), props)
                        .expect("oMLX GQL bench query anchor inserts");
                    for document in documents
                        .iter()
                        .filter(|document| document.topic == input.topic && document.graph_hint)
                    {
                        mutator
                            .create_edge(
                                dependency_edge.clone(),
                                anchor,
                                document.node,
                                PropertyMap::new(),
                            )
                            .expect("oMLX GQL bench query dependency edge inserts");
                    }
                    query_anchors.push(input.topic);
                }
            }
            txn.commit().expect("oMLX GQL bench graph commits");
        }
        let mut query_anchors = query_anchors.into_iter();
        let queries = inputs
            .iter()
            .zip(vectors)
            .filter_map(|(input, vector)| {
                (!input.is_document).then(|| {
                    let anchor_topic = query_anchors
                        .next()
                        .expect("oMLX GQL bench query anchor count matches queries");
                    debug_assert_eq!(anchor_topic, input.topic);
                    QueryVector {
                        topic: input.topic,
                        vector,
                    }
                })
            })
            .collect::<Vec<_>>();
        debug_assert!(
            query_anchors.next().is_none(),
            "oMLX GQL bench consumed every query anchor"
        );
        let topics_by_node = documents
            .iter()
            .map(|document| (document.node, document.topic))
            .collect();
        let current_by_node = documents
            .iter()
            .map(|document| (document.node, document.current_fact))
            .collect();
        Self {
            graph,
            dimension,
            documents,
            topics_by_node,
            current_by_node,
            queries,
        }
    }

    pub(super) fn query_count(&self) -> usize {
        self.queries.len()
    }

    pub(super) fn first_query_root_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.documents
                .iter()
                .filter(|document| document.topic == query.topic && document.graph_hint)
                .count()
        })
    }

    pub(super) fn first_query_expanded_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            if self.first_query_root_count() == 0 {
                0
            } else {
                self.documents
                    .iter()
                    .filter(|document| document.topic == query.topic)
                    .count()
            }
        })
    }

    pub(super) fn first_query_state_intersection_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            if self.first_query_root_count() == 0 {
                0
            } else {
                self.documents
                    .iter()
                    .filter(|document| document.topic == query.topic && document.support_fact)
                    .count()
            }
        })
    }

    pub(super) fn first_query_current_state_intersection_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            if self.first_query_root_count() == 0 {
                0
            } else {
                self.documents
                    .iter()
                    .filter(|document| {
                        document.topic == query.topic
                            && document.support_fact
                            && document.current_fact
                    })
                    .count()
            }
        })
    }

    pub(super) fn warm_query_root_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_query(0, registry, Some(cache));
    }

    pub(super) fn warm_query_root_state_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_state_query(0, registry, Some(cache));
    }

    pub(super) fn warm_query_root_current_state_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_current_state_query(0, registry, Some(cache));
    }

    pub(super) fn warm_query_root_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_batch_query(registry, Some(cache));
    }

    pub(super) fn execute_all_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    pub(super) fn execute_all_state_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_state_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    pub(super) fn execute_all_current_state_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_current_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                )
                .row_count()
            })
            .sum()
    }

    pub(super) fn gql_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(super) fn gql_current_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.current_precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(super) fn gql_state_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_state_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(super) fn gql_current_state_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table = self.execute_current_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                self.current_precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(super) fn gql_batch_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_batch_query(registry, cache);
        precision_basis_points(self.batch_precision(&table), self.query_count() * TOP_K)
    }

    fn execute_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session
            .execute_source(QUERY_ROOT_SOURCE, registry)
            .expect("oMLX GQL query-root vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_state_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session
            .execute_source(QUERY_ROOT_STATE_SOURCE, registry)
            .expect("oMLX GQL query-root state vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_current_state_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session
            .execute_source(QUERY_ROOT_CURRENT_STATE_SOURCE, registry)
            .expect("oMLX GQL query-root current-state vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    pub(super) fn execute_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session
            .execute_source(QUERY_ROOT_BATCH_SOURCE, registry)
            .expect("oMLX GQL batched query-root vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn precision(&self, topic: Topic, table: &BindingTable) -> usize {
        let node_column = table
            .column_index(istr("node_id"))
            .expect("node_id column exists");
        table
            .iter()
            .filter_map(|row| match row.get(node_column) {
                Some(Value::NodeRef(node)) => Some(*node),
                _ => None,
            })
            .filter(|node| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| *hit_topic == topic)
            })
            .count()
    }

    fn current_precision(&self, topic: Topic, table: &BindingTable) -> usize {
        let node_column = table
            .column_index(istr("node_id"))
            .expect("node_id column exists");
        table
            .iter()
            .filter_map(|row| match row.get(node_column) {
                Some(Value::NodeRef(node)) => Some(*node),
                _ => None,
            })
            .filter(|node| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| *hit_topic == topic)
                    && self.current_by_node.get(node).copied().unwrap_or(false)
            })
            .count()
    }

    fn batch_precision(&self, table: &BindingTable) -> usize {
        let query_column = table
            .column_index(istr("query_index"))
            .expect("query_index column exists");
        let node_column = table
            .column_index(istr("node_id"))
            .expect("node_id column exists");
        table
            .iter()
            .filter_map(|row| match (row.get(query_column), row.get(node_column)) {
                (Some(Value::Uint(query_index)), Some(Value::NodeRef(node))) => {
                    let query_index = usize::try_from(*query_index).ok()?;
                    let topic = self.queries.get(query_index)?.topic;
                    Some((topic, *node))
                }
                _ => None,
            })
            .filter(|(topic, node)| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| hit_topic == topic)
            })
            .count()
    }
}

#[derive(Clone, Copy)]
struct DocumentMeta {
    node: NodeId,
    topic: Topic,
    graph_hint: bool,
    support_fact: bool,
    current_fact: bool,
}

struct QueryVector {
    topic: Topic,
    vector: VectorValue,
}

fn graph_id_for_model(model: &str) -> GraphId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    GraphId::new(98_000 + hasher.finish() % 1_000)
}

fn admits_graph_hint(
    graph_hint_counts: &mut HashMap<Topic, usize>,
    topic: Topic,
    graph_hint_docs_per_topic: Option<usize>,
) -> bool {
    let Some(limit) = graph_hint_docs_per_topic else {
        return true;
    };
    let count = graph_hint_counts.entry(topic).or_insert(0);
    if *count >= limit {
        return false;
    }
    *count += 1;
    true
}

fn is_negative_evidence_document(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["stale", "superseded", "contradict"]
        .iter()
        .any(|needle| text.contains(needle))
}

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}
