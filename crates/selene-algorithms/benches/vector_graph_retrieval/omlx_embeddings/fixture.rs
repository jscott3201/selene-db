//! Graph fixture built from local oMLX endpoint embeddings.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use selene_core::{
    CancellationChecker, GraphId, HnswIndexConfig, LabelSet, NodeId, PropertyMap, Value,
    VectorMetric, VectorValue,
};
use selene_graph::{
    ApproximateVectorSearchOptions, RowIndex, SeleneGraph, SharedGraph, VectorCandidateSet,
    VectorIndexConfig, VectorIndexKind, VectorNeighborDirection, VectorNeighborSearchOptions,
};

use super::super::support::istr;
use super::corpus::{CorpusInput, Topic, topic_label};
use super::{ANN_SEARCH_WIDTH, TOP_K, precision_basis_points};

pub(super) struct OmlxVectorFixture {
    graph: SeleneGraph,
    label: selene_core::IStr,
    embedding_key: selene_core::IStr,
    dependency_edge: selene_core::IStr,
    support_edge: selene_core::IStr,
    pub(super) dimension: usize,
    documents: Vec<DocumentMeta>,
    topics_by_node: HashMap<NodeId, Topic>,
    queries: Vec<QueryVector>,
    expanded_hint_sets: Vec<VectorCandidateSet>,
}

impl OmlxVectorFixture {
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
        let label = istr("OmlxEmbeddingDoc");
        let query_label = istr("OmlxQueryAnchor");
        let dependency_edge = istr("OmlxDependsOn");
        let support_edge = istr("OmlxSupports");
        let embedding_key = istr("embedding");
        let shared = SharedGraph::new(graph_id_for_model(model));
        let mut documents = Vec::new();
        let mut query_anchors = Vec::new();
        let mut graph_hint_counts = HashMap::<Topic, usize>::new();
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
                    let graph_hint = admits_graph_hint(
                        &mut graph_hint_counts,
                        input.topic,
                        graph_hint_docs_per_topic,
                    );
                    let mut labels = LabelSet::single(label.clone());
                    if graph_hint {
                        labels.insert(topic_label(input.topic));
                    }
                    let node = mutator
                        .create_node(labels, props)
                        .expect("oMLX bench document node inserts");
                    documents.push(DocumentMeta {
                        node,
                        topic: input.topic,
                        graph_hint,
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
                            .expect("oMLX bench support edge inserts");
                    }
                }
                for input in inputs.iter().filter(|input| !input.is_document) {
                    let anchor = mutator
                        .create_node(
                            LabelSet::from_iter([query_label.clone()]),
                            PropertyMap::new(),
                        )
                        .expect("oMLX bench query anchor inserts");
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
                            .expect("oMLX bench query dependency edge inserts");
                    }
                    query_anchors.push(QueryAnchor {
                        node: anchor,
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
        let mut query_anchors = query_anchors.into_iter();
        let queries: Vec<QueryVector> = inputs
            .iter()
            .zip(vectors)
            .filter_map(|(input, vector)| {
                (!input.is_document).then(|| {
                    let anchor = query_anchors
                        .next()
                        .expect("oMLX bench query anchor count matches queries");
                    debug_assert_eq!(anchor.topic, input.topic);
                    QueryVector {
                        anchor: anchor.node,
                        topic: input.topic,
                        vector,
                    }
                })
            })
            .collect();
        debug_assert!(
            query_anchors.next().is_none(),
            "oMLX bench consumed every query anchor"
        );
        let graph = shared.read().as_ref().clone();
        let expanded_hint_sets = queries
            .iter()
            .map(|query| {
                topic_hint_expansion_set_for(&graph, &dependency_edge, &support_edge, query.anchor)
            })
            .collect();
        let topics_by_node = documents
            .iter()
            .map(|document| (document.node, document.topic))
            .collect();
        Self {
            graph,
            label,
            embedding_key,
            dependency_edge,
            support_edge,
            dimension,
            documents,
            topics_by_node,
            queries,
            expanded_hint_sets,
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

    pub(super) fn topic_neighbor_total_precision(&self) -> usize {
        let options = self.neighbor_options(TOP_K);
        self.queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .score_vector_neighbors_checked(
                        &self.embedding_key,
                        &query.vector,
                        query.anchor,
                        options,
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX topic neighbor scoring succeeds");
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    pub(super) fn topic_neighbor_batch_total_precision(&self) -> usize {
        let queries = self
            .queries
            .iter()
            .map(|query| query.vector.clone())
            .collect::<Vec<_>>();
        let anchors = self
            .queries
            .iter()
            .map(|query| query.anchor)
            .collect::<Vec<_>>();
        let hits = self
            .graph
            .score_vector_neighbors_batch_checked(
                &self.embedding_key,
                &queries,
                &anchors,
                self.neighbor_options(TOP_K),
                CancellationChecker::disabled(),
            )
            .expect("oMLX topic neighbor batch scoring succeeds");
        self.queries
            .iter()
            .zip(hits)
            .map(|(query, hits)| {
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    pub(super) fn topic_label_ann_union_total_precision(&self) -> usize {
        self.candidate_sets_total_precision(|fixture, query| {
            fixture
                .topic_candidate_set(query.topic)
                .union(&fixture.ann_hit_set(query, super::ANN_UNION_SEED_K))
        })
    }

    pub(super) fn topic_neighbor_ann_union_total_precision(&self) -> usize {
        self.candidate_sets_total_precision(|fixture, query| {
            fixture
                .topic_neighbor_set(query)
                .union(&fixture.ann_hit_set(query, super::ANN_UNION_SEED_K))
        })
    }

    pub(super) fn topic_hint_expansion_total_precision(&self) -> usize {
        self.candidate_sets_total_precision(|fixture, query| {
            fixture.topic_hint_expansion_set(query)
        })
    }

    pub(super) fn topic_hint_expansion_ann_union_total_precision(&self) -> usize {
        self.candidate_sets_total_precision(|fixture, query| {
            fixture
                .topic_hint_expansion_set(query)
                .union(&fixture.ann_hit_set(query, super::ANN_UNION_SEED_K))
        })
    }

    pub(super) fn topic_hint_expansion_cached_total_precision(&self) -> usize {
        self.candidate_sets_total_precision_from_sets(&self.expanded_hint_sets)
    }

    pub(super) fn topic_candidate_count(&self) -> usize {
        self.topic_candidate_set(Topic::Gql).len()
    }

    pub(super) fn topic_neighbor_count(&self) -> usize {
        self.queries
            .first()
            .map_or(0, |query| self.topic_neighbor_set(query).len())
    }

    pub(super) fn topic_label_ann_union_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.topic_candidate_set(query.topic)
                .union(&self.ann_hit_set(query, super::ANN_UNION_SEED_K))
                .len()
        })
    }

    pub(super) fn topic_neighbor_ann_union_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.topic_neighbor_set(query)
                .union(&self.ann_hit_set(query, super::ANN_UNION_SEED_K))
                .len()
        })
    }

    pub(super) fn topic_hint_expansion_count(&self) -> usize {
        self.queries
            .first()
            .map_or(0, |query| self.topic_hint_expansion_set(query).len())
    }

    pub(super) fn topic_hint_expansion_ann_union_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.topic_hint_expansion_set(query)
                .union(&self.ann_hit_set(query, super::ANN_UNION_SEED_K))
                .len()
        })
    }

    pub(super) fn topic_hint_expansion_cached_count(&self) -> usize {
        self.expanded_hint_sets
            .first()
            .map_or(0, VectorCandidateSet::len)
    }

    fn candidate_sets_total_precision<F>(&self, candidate_set_for: F) -> usize
    where
        F: Fn(&Self, &QueryVector) -> VectorCandidateSet,
    {
        let candidate_sets = self
            .queries
            .iter()
            .map(|query| candidate_set_for(self, query))
            .collect::<Vec<_>>();
        self.candidate_sets_total_precision_from_sets(&candidate_sets)
    }

    fn candidate_sets_total_precision_from_sets(
        &self,
        candidate_sets: &[VectorCandidateSet],
    ) -> usize {
        let queries = self
            .queries
            .iter()
            .map(|query| query.vector.clone())
            .collect::<Vec<_>>();
        let hits = self
            .graph
            .score_vector_candidate_sets_batch_checked(
                &self.embedding_key,
                &queries,
                candidate_sets,
                VectorMetric::Cosine,
                TOP_K,
                CancellationChecker::disabled(),
            )
            .expect("oMLX ANN-union candidate scoring succeeds");
        self.queries
            .iter()
            .zip(hits)
            .map(|(query, hits)| {
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
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

    fn topic_neighbor_set(&self, query: &QueryVector) -> VectorCandidateSet {
        self.graph.vector_neighbor_candidates(
            query.anchor,
            &self.dependency_edge,
            VectorNeighborDirection::Outgoing,
        )
    }

    fn topic_hint_expansion_set(&self, query: &QueryVector) -> VectorCandidateSet {
        topic_hint_expansion_set_for(
            &self.graph,
            &self.dependency_edge,
            &self.support_edge,
            query.anchor,
        )
    }

    fn ann_hit_set(&self, query: &QueryVector, k: usize) -> VectorCandidateSet {
        let hits = self
            .graph
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &query.vector,
                ApproximateVectorSearchOptions::new(VectorMetric::Cosine, k, ANN_SEARCH_WIDTH),
                CancellationChecker::disabled(),
            )
            .expect("oMLX ANN hit-set search succeeds");
        VectorCandidateSet::from_search_hits(hits)
    }

    fn neighbor_options(&self, k: usize) -> VectorNeighborSearchOptions<'_> {
        VectorNeighborSearchOptions::new(
            &self.dependency_edge,
            VectorNeighborDirection::Outgoing,
            VectorMetric::Cosine,
            k,
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

fn topic_hint_expansion_set_for(
    graph: &SeleneGraph,
    dependency_edge: &selene_core::IStr,
    support_edge: &selene_core::IStr,
    anchor: NodeId,
) -> VectorCandidateSet {
    let roots = graph.vector_neighbor_candidates(
        anchor,
        dependency_edge,
        VectorNeighborDirection::Outgoing,
    );
    let mut expanded = roots.clone();
    for root in roots.as_nodes() {
        expanded = expanded.union(&graph.vector_neighbor_candidates(
            *root,
            support_edge,
            VectorNeighborDirection::Outgoing,
        ));
    }
    expanded
}

struct DocumentMeta {
    node: NodeId,
    topic: Topic,
    graph_hint: bool,
}

struct QueryAnchor {
    node: NodeId,
    topic: Topic,
}

struct QueryVector {
    anchor: NodeId,
    topic: Topic,
    vector: VectorValue,
}

fn graph_id_for_model(model: &str) -> GraphId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    GraphId::new(97_000 + hasher.finish() % 1_000)
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
