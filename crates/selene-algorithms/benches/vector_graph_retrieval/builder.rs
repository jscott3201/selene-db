//! Fixture construction for graph/vector retrieval benchmarks.

use std::collections::HashMap;

use selene_core::{HnswIndexConfig, LabelSet, PropertyMap, Value};
use selene_graph::{SharedGraph, VectorIndexConfig, VectorIndexKind};

use super::support::{
    DIMENSION, FACTS_PER_TOPIC, component_candidates, current_replacement, duplicates_per_fact,
    graph_id_for_scale, istr, memory_vector, pagerank_scores, topic_count,
};
use super::{MemoryRetrievalFixture, NodeMeta, Query, TopologyNoise, support};

impl MemoryRetrievalFixture {
    pub(super) fn build(requested_scale: usize) -> Self {
        Self::build_with_topology(requested_scale, TopologyNoise::Clean)
    }

    pub(super) fn build_with_topology(requested_scale: usize, topology: TopologyNoise) -> Self {
        let label = istr("Memory");
        let scope_label = istr("MemoryScope");
        let embedding_key = istr("embedding");
        let support_edge = istr("SUPPORTS");
        let scope_edge = istr("IN_SCOPE");
        let valid_edge = istr("VALID_AT");
        let superseded_by_edge = istr("SUPERSEDED_BY");
        let topic_count = topic_count(requested_scale);
        let duplicates = duplicates_per_fact(requested_scale, topic_count);
        let shared = SharedGraph::new(graph_id_for_scale(requested_scale));
        let mut topic_nodes = vec![vec![Vec::new(); FACTS_PER_TOPIC]; topic_count];
        let mut metadata = HashMap::new();

        {
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                let scope_nodes: Vec<_> = (0..topic_count)
                    .map(|_| {
                        mutator
                            .create_node(LabelSet::single(scope_label.clone()), PropertyMap::new())
                            .expect("bench scope node insert succeeds")
                    })
                    .collect();
                for (topic, facts) in topic_nodes.iter_mut().enumerate() {
                    for (fact, nodes) in facts.iter_mut().enumerate() {
                        for duplicate in 0..duplicates {
                            let vector = memory_vector(topic, fact, duplicate, 0.0);
                            let props = PropertyMap::from_pairs([(
                                embedding_key.clone(),
                                Value::Vector(vector),
                            )])
                            .expect("bench properties fit");
                            let node = mutator
                                .create_node(LabelSet::single(label.clone()), props)
                                .expect("bench node insert succeeds");
                            nodes.push(node);
                            metadata.insert(
                                node,
                                NodeMeta {
                                    topic,
                                    fact,
                                    current: fact == 0 || duplicate % 2 == 0,
                                },
                            );
                        }
                    }
                }
                for (topic, facts) in topic_nodes.iter().enumerate() {
                    let scope = scope_nodes[topic];
                    for nodes in facts {
                        for &node in nodes {
                            mutator
                                .create_edge(scope_edge.clone(), node, scope, PropertyMap::new())
                                .expect("bench scope edge inserts");
                        }
                    }
                }
                for facts in &topic_nodes {
                    for (duplicate, summary) in facts[0].iter().enumerate() {
                        for evidence_nodes in facts.iter().skip(1) {
                            let evidence = evidence_nodes[duplicate % evidence_nodes.len()];
                            mutator
                                .create_edge(
                                    support_edge.clone(),
                                    *summary,
                                    evidence,
                                    PropertyMap::new(),
                                )
                                .expect("bench support edge inserts");
                            if metadata.get(&evidence).is_some_and(|meta| meta.current) {
                                mutator
                                    .create_edge(
                                        valid_edge.clone(),
                                        *summary,
                                        evidence,
                                        PropertyMap::new(),
                                    )
                                    .expect("bench valid edge inserts");
                            } else {
                                let replacement = current_replacement(evidence_nodes, duplicate);
                                mutator
                                    .create_edge(
                                        superseded_by_edge.clone(),
                                        evidence,
                                        replacement,
                                        PropertyMap::new(),
                                    )
                                    .expect("bench supersession edge inserts");
                            }
                        }
                    }
                }
                if topology == TopologyNoise::CrossTopicSupportRing {
                    for topic in 0..topic_count {
                        let next_topic = (topic + 1) % topic_count;
                        for duplicate in 0..duplicates {
                            let summary = topic_nodes[topic][0][duplicate];
                            let target_fact = 1 + duplicate % (FACTS_PER_TOPIC - 1);
                            let target_duplicate = (duplicate + 1) % duplicates;
                            let target = topic_nodes[next_topic][target_fact][target_duplicate];
                            mutator
                                .create_edge(
                                    support_edge.clone(),
                                    summary,
                                    target,
                                    PropertyMap::new(),
                                )
                                .expect("bench noisy support edge inserts");
                        }
                    }
                }
                mutator
                    .create_vector_index_named_with_configs(
                        label.clone(),
                        embedding_key.clone(),
                        VectorIndexKind::HnswCosine,
                        DIMENSION as u32,
                        None,
                        VectorIndexConfig::new(Some(HnswIndexConfig::new(24, 64)), None),
                    )
                    .expect("bench vector index build succeeds");
            }
            txn.commit().expect("bench graph commits");
        }

        let graph = shared.read().as_ref().clone();
        let pagerank = pagerank_scores(&graph, &label, &support_edge);
        let (component_by_node, component_candidates) =
            component_candidates(&graph, &label, &support_edge, &superseded_by_edge);
        let mut component_order: Vec<_> = component_candidates.keys().copied().collect();
        component_order.sort_unstable();
        let component_offsets = component_order
            .iter()
            .copied()
            .enumerate()
            .map(|(offset, component)| (component, offset))
            .collect();
        let mut topic_candidates = vec![Vec::new(); topic_count];
        for (&node, meta) in &metadata {
            topic_candidates[meta.topic].push(node);
        }
        for candidates in &mut topic_candidates {
            candidates.sort_unstable();
        }
        let queries = (0..topic_count)
            .map(|topic| {
                let anchor = topic_nodes[topic][0][duplicates / 2];
                Query {
                    topic,
                    anchor,
                    component: component_by_node[&anchor],
                    vector: memory_vector(topic, 0, duplicates / 2, 0.0003),
                }
            })
            .collect();
        Self {
            graph,
            scale: topic_count * FACTS_PER_TOPIC * duplicates,
            label,
            embedding_key,
            support_edge,
            scope_edge,
            valid_edge,
            superseded_by_edge,
            queries,
            metadata,
            pagerank,
            component_candidates,
            component_order,
            component_offsets,
            louvain_by_node: HashMap::new(),
            louvain_candidates: HashMap::new(),
            label_by_node: HashMap::new(),
            label_candidates: HashMap::new(),
            topic_candidates,
        }
    }

    pub(super) fn build_with_community_topology(
        requested_scale: usize,
        topology: TopologyNoise,
    ) -> Self {
        let mut fixture = Self::build_with_topology(requested_scale, topology);
        let (louvain_by_node, louvain_candidates) = support::louvain_candidates(
            &fixture.graph,
            &fixture.label,
            &fixture.support_edge,
            &fixture.superseded_by_edge,
        );
        let (label_by_node, label_candidates) = support::label_propagation_candidates(
            &fixture.graph,
            &fixture.label,
            &fixture.support_edge,
            &fixture.superseded_by_edge,
        );
        fixture.louvain_by_node = louvain_by_node;
        fixture.louvain_candidates = louvain_candidates;
        fixture.label_by_node = label_by_node;
        fixture.label_candidates = label_candidates;
        fixture
    }
}
