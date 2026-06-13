use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use selene_core::{
    DbString, GraphId, JsonValue, LabelSet, NodeId, PropertyMap, Value, VectorValue,
};
use selene_gql::BindingTable;
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
};
use selene_testing::local_omlx::{CorpusInput, Topic, topic_label};

#[path = "fixture/json_exec.rs"]
mod json_exec;
#[path = "fixture/query_exec.rs"]
mod query_exec;
#[path = "fixture/rrf_exec.rs"]
mod rrf_exec;
#[path = "fixture/text_exec.rs"]
mod text_exec;

pub(super) const TOP_K: usize = 4;

pub(super) struct OmlxGqlQueryRootFixture {
    graph: SharedGraph,
    pub(super) dimension: usize,
    documents: Vec<DocumentMeta>,
    topics_by_node: HashMap<NodeId, Topic>,
    current_by_node: HashMap<NodeId, bool>,
    target_by_node: HashMap<NodeId, &'static str>,
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
        let doc_label = db_string("OmlxEmbeddingDoc");
        let query_label = db_string("OmlxQueryAnchor");
        let support_fact_label = db_string("OmlxSupportFact");
        let provenance_label = db_string("OmlxEvidenceSource");
        let dependency_edge = db_string("OmlxDependsOn");
        let support_edge = db_string("OmlxSupports");
        let negative_evidence_edge = db_string("OmlxNegativeEvidence");
        let provenance_edge = db_string("OmlxGroundedBy");
        let embedding_key = db_string("embedding");
        let body_key = db_string("body");
        let metadata_key = db_string("metadata");
        let query_key = db_string("query");
        let query_text_key = db_string("query_text");
        let query_index_key = db_string("query_index");
        let support_state_provider = Arc::new(
            MaintainedCandidateStateProvider::new([
                CandidateStateSpec::new(db_string("omlx_support_facts"))
                    .require_label(support_fact_label.clone()),
                CandidateStateSpec::new(db_string("omlx_current_support_facts"))
                    .require_label(support_fact_label.clone())
                    .exclude_outgoing(negative_evidence_edge.clone()),
                CandidateStateSpec::new(db_string("omlx_provenance_current_support_facts"))
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
                    let current_fact = !is_negative_evidence_document(input.text());
                    let metadata = document_metadata(input, graph_hint, support_fact, current_fact);
                    let props = PropertyMap::from_pairs([
                        (embedding_key.clone(), Value::Vector(vector.clone())),
                        (body_key.clone(), Value::String(db_string(input.text()))),
                        (metadata_key.clone(), metadata),
                    ])
                    .expect("oMLX GQL bench document properties fit");
                    let node = mutator
                        .create_node(labels, props)
                        .expect("oMLX GQL bench document node inserts");
                    documents.push(DocumentMeta {
                        node,
                        topic: input.topic,
                        graph_hint,
                        support_fact,
                        current_fact,
                        target_key: input.target_key,
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
                        (
                            query_text_key.clone(),
                            Value::String(db_string(input.text())),
                        ),
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
        graph
            .create_text_index(doc_label.clone(), body_key)
            .expect("oMLX GQL bench text index registers");
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
                        text: input.text().to_owned(),
                        vector,
                        target_key: input.target_key,
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
        let target_by_node = documents
            .iter()
            .filter_map(|document| document.target_key.map(|target| (document.node, target)))
            .collect();
        Self {
            graph,
            dimension,
            documents,
            topics_by_node,
            current_by_node,
            target_by_node,
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

    pub(super) fn target_hit_basis_points(&self, hits: usize) -> Option<usize> {
        let target_queries = self
            .queries
            .iter()
            .filter(|query| query.target_key.is_some())
            .count();
        (target_queries > 0).then(|| basis_points(hits, target_queries))
    }

    pub(super) fn target_hit_count(&self, query_index: usize, table: &BindingTable) -> usize {
        let Some(expected) = self
            .queries
            .get(query_index)
            .and_then(|query| query.target_key)
        else {
            return 0;
        };
        let node_column = table
            .column_index(db_string("node_id"))
            .expect("node_id column exists");
        table.iter().any(|row| match row.get(node_column) {
            Some(Value::NodeRef(node)) => self.target_by_node.get(node) == Some(&expected),
            _ => false,
        }) as usize
    }

    pub(super) fn batch_target_hit_count(&self, table: &BindingTable) -> usize {
        let query_column = table
            .column_index(db_string("query_index"))
            .expect("query_index column exists");
        let node_column = table
            .column_index(db_string("node_id"))
            .expect("node_id column exists");
        let mut hits = vec![false; self.queries.len()];
        for row in table.iter() {
            let (Some(Value::Uint(query_index)), Some(Value::NodeRef(node))) =
                (row.get(query_column), row.get(node_column))
            else {
                continue;
            };
            let Ok(query_index) = usize::try_from(*query_index) else {
                continue;
            };
            let Some(expected) = self
                .queries
                .get(query_index)
                .and_then(|query| query.target_key)
            else {
                continue;
            };
            if self.target_by_node.get(node) == Some(&expected) {
                hits[query_index] = true;
            }
        }
        hits.into_iter().filter(|hit| *hit).count()
    }
}

#[derive(Clone, Copy)]
struct DocumentMeta {
    node: NodeId,
    topic: Topic,
    graph_hint: bool,
    support_fact: bool,
    current_fact: bool,
    target_key: Option<&'static str>,
}

struct QueryVector {
    topic: Topic,
    text: String,
    vector: VectorValue,
    target_key: Option<&'static str>,
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

fn document_metadata(
    input: &CorpusInput,
    graph_hint: bool,
    support_fact: bool,
    current_fact: bool,
) -> Value {
    Value::Json(
        JsonValue::new(serde_json::json!({
            "retrieval": {
                "topic": topic_name(input.topic),
                "current": current_fact,
                "support": support_fact,
                "graph_hint": graph_hint,
                "target": input.target_key,
            }
        }))
        .expect("embedding benchmark metadata JSON is valid"),
    )
}

const fn topic_name(topic: Topic) -> &'static str {
    match topic {
        Topic::Gql => "gql",
        Topic::Vector => "vector",
        Topic::AgentMemory => "agent_memory",
        Topic::Code => "code",
    }
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("bench string fits DB string cap")
}

fn basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}
