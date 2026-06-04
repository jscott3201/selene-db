//! Graph fixture built from local oMLX endpoint embeddings.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use selene_core::{
    CancellationChecker, GraphId, HnswIndexConfig, LabelSet, NodeId, PropertyMap, Value,
    VectorMetric, VectorValue,
};
use selene_graph::{
    ApproximateVectorSearchOptions, RowIndex, SeleneGraph, SharedGraph, VectorCandidateSet,
    VectorIndexConfig, VectorIndexKind,
};

use super::super::support::istr;
use super::corpus::{CorpusInput, Topic, topic_label};
use super::{ANN_SEARCH_WIDTH, TOP_K, precision_basis_points};

pub(super) struct OmlxVectorFixture {
    graph: SeleneGraph,
    label: selene_core::IStr,
    embedding_key: selene_core::IStr,
    pub(super) dimension: usize,
    documents: Vec<DocumentMeta>,
    topics_by_node: HashMap<NodeId, Topic>,
    queries: Vec<QueryVector>,
}

impl OmlxVectorFixture {
    pub(super) fn build(model: &str, inputs: &[CorpusInput], vectors: Vec<VectorValue>) -> Self {
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
        let label = istr("OmlxEmbeddingDoc");
        let embedding_key = istr("embedding");
        let shared = SharedGraph::new(graph_id_for_model(model));
        let mut documents = Vec::new();
        {
            let mut txn = shared.begin_write();
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
                    .expect("oMLX bench document properties fit");
                    let node = mutator
                        .create_node(
                            LabelSet::from_iter([label.clone(), topic_label(input.topic)]),
                            props,
                        )
                        .expect("oMLX bench document node inserts");
                    documents.push(DocumentMeta {
                        node,
                        topic: input.topic,
                    });
                }
                mutator
                    .create_vector_index_named_with_configs(
                        label.clone(),
                        embedding_key.clone(),
                        VectorIndexKind::HnswCosine,
                        dimension as u32,
                        None,
                        VectorIndexConfig::new(Some(HnswIndexConfig::new(16, 64)), None),
                    )
                    .expect("oMLX bench HNSW index builds");
            }
            txn.commit().expect("oMLX bench graph commits");
        }
        let queries = inputs
            .iter()
            .zip(vectors)
            .filter_map(|(input, vector)| {
                (!input.is_document).then_some(QueryVector {
                    topic: input.topic,
                    vector,
                })
            })
            .collect();
        let topics_by_node = documents
            .iter()
            .map(|document| (document.node, document.topic))
            .collect();
        Self {
            graph: shared.read().as_ref().clone(),
            label,
            embedding_key,
            dimension,
            documents,
            topics_by_node,
            queries,
        }
    }

    pub(super) fn document_count(&self) -> usize {
        self.documents.len()
    }

    pub(super) fn query_count(&self) -> usize {
        self.queries.len()
    }

    pub(super) fn exact_total_precision(&self) -> usize {
        self.queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .exact_vector_search_nodes_checked(
                        &self.label,
                        &self.embedding_key,
                        &query.vector,
                        VectorMetric::Cosine,
                        TOP_K,
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX exact vector search succeeds");
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    pub(super) fn ann_total_precision(&self) -> usize {
        self.queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .approximate_vector_search_nodes_checked(
                        &self.label,
                        &self.embedding_key,
                        &query.vector,
                        ApproximateVectorSearchOptions::new(
                            VectorMetric::Cosine,
                            TOP_K,
                            ANN_SEARCH_WIDTH,
                        ),
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX ANN vector search succeeds");
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    pub(super) fn exact_precision_basis_points(&self) -> usize {
        precision_basis_points(self.exact_total_precision(), self.query_count() * TOP_K)
    }

    pub(super) fn ann_precision_basis_points(&self) -> usize {
        precision_basis_points(self.ann_total_precision(), self.query_count() * TOP_K)
    }

    pub(super) fn topic_candidate_total_precision(&self) -> usize {
        let queries = self
            .queries
            .iter()
            .map(|query| query.vector.clone())
            .collect::<Vec<_>>();
        let candidate_sets = self
            .queries
            .iter()
            .map(|query| self.topic_candidate_set(query.topic))
            .collect::<Vec<_>>();
        let hits = self
            .graph
            .score_vector_candidate_sets_batch_checked(
                &self.embedding_key,
                &queries,
                &candidate_sets,
                VectorMetric::Cosine,
                TOP_K,
                CancellationChecker::disabled(),
            )
            .expect("oMLX topic candidate scoring succeeds");
        self.queries
            .iter()
            .zip(hits)
            .map(|(query, hits)| {
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    pub(super) fn topic_candidate_count(&self) -> usize {
        self.topic_candidate_set(Topic::Gql).len()
    }

    fn topic_candidate_set(&self, topic: Topic) -> VectorCandidateSet {
        let topic_label = topic_label(topic);
        let Some(rows) = self.graph.nodes_with_label(&topic_label) else {
            return VectorCandidateSet::default();
        };
        VectorCandidateSet::from_nodes(
            rows.iter()
                .filter_map(|row| self.graph.node_id_for_row(RowIndex::new(row))),
        )
    }

    fn precision<I>(&self, topic: Topic, hits: I) -> usize
    where
        I: IntoIterator<Item = NodeId>,
    {
        hits.into_iter()
            .filter(|node| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| *hit_topic == topic)
            })
            .count()
    }
}

struct DocumentMeta {
    node: NodeId,
    topic: Topic,
}

struct QueryVector {
    topic: Topic,
    vector: VectorValue,
}

fn graph_id_for_model(model: &str) -> GraphId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    GraphId::new(97_000 + hasher.finish() % 1_000)
}
