use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
};
use selene_testing::local_omlx::{CorpusInput, Topic, topic_label};

#[path = "fixture/query_exec.rs"]
mod query_exec;

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
        let provenance_label = istr("OmlxEvidenceSource");
        let dependency_edge = istr("OmlxDependsOn");
        let support_edge = istr("OmlxSupports");
        let negative_evidence_edge = istr("OmlxNegativeEvidence");
        let provenance_edge = istr("OmlxGroundedBy");
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
                CandidateStateSpec::new(istr("omlx_provenance_current_support_facts"))
                    .require_label(support_fact_label.clone())
                    .require_incoming(support_edge.clone())
                    .require_outgoing(provenance_edge.clone())
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
                for document in documents
                    .iter()
                    .filter(|document| document.support_fact && document.current_fact)
                {
                    let provenance = mutator
                        .create_node(
                            LabelSet::single(provenance_label.clone()),
                            PropertyMap::new(),
                        )
                        .expect("oMLX GQL bench provenance node inserts");
                    mutator
                        .create_edge(
                            provenance_edge.clone(),
                            document.node,
                            provenance,
                            PropertyMap::new(),
                        )
                        .expect("oMLX GQL bench provenance edge inserts");
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

    pub(super) fn first_query_provenance_state_intersection_count(&self) -> usize {
        self.first_query_current_state_intersection_count()
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

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}
