//! Fixture construction for graph/vector retrieval benchmarks.

use std::collections::{HashMap, HashSet};

use selene_core::{DbString, HnswIndexConfig, LabelSet, NodeId, PropertyMap, Value};
use selene_graph::{SeleneGraph, SharedGraph, VectorIndexConfig, VectorIndexKind};

use super::support::{
    DIMENSION, FACTS_PER_TOPIC, SEED_K, TOPICS_PER_SESSION, component_candidates,
    current_replacement, db_string, duplicates_per_fact, graph_id_for_scale, memory_vector,
    pagerank_scores, topic_count,
};
use super::{MemoryRetrievalFixture, NodeMeta, Query, TopologyNoise, support};

impl MemoryRetrievalFixture {
    pub(super) fn build(requested_scale: usize) -> Self {
        Self::build_with_topology(requested_scale, TopologyNoise::Clean)
    }

    pub(super) fn build_with_topology(requested_scale: usize, topology: TopologyNoise) -> Self {
        let label = db_string("Memory");
        let bridge_label = db_string("MemoryBridge");
        let recent_label = db_string("MemoryRecentWindow");
        let scope_label = db_string("MemoryScope");
        let session_label = db_string("MemorySession");
        let embedding_key = db_string("embedding");
        let support_edge = db_string("SUPPORTS");
        let scope_edge = db_string("IN_SCOPE");
        let session_edge = db_string("IN_SESSION");
        let valid_edge = db_string("VALID_AT");
        let superseded_by_edge = db_string("SUPERSEDED_BY");
        let contradicts_edge = db_string("CONTRADICTS");
        let recent_edge = db_string("RECENT_IN");
        let depends_edge = db_string("DEPENDS_ON");
        let topic_count = topic_count(requested_scale);
        let session_count = topic_count.div_ceil(TOPICS_PER_SESSION);
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
                let session_nodes: Vec<_> = (0..session_count)
                    .map(|_| {
                        mutator
                            .create_node(
                                LabelSet::single(session_label.clone()),
                                PropertyMap::new(),
                            )
                            .expect("bench session node insert succeeds")
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
                    let session = session_nodes[topic / TOPICS_PER_SESSION];
                    for nodes in facts {
                        for &node in nodes {
                            mutator
                                .create_edge(scope_edge.clone(), node, scope, PropertyMap::new())
                                .expect("bench scope edge inserts");
                            mutator
                                .create_edge(
                                    session_edge.clone(),
                                    node,
                                    session,
                                    PropertyMap::new(),
                                )
                                .expect("bench session edge inserts");
                        }
                    }
                }
                if matches!(
                    topology,
                    TopologyNoise::NoisySparseSupport
                        | TopologyNoise::NoisyMultiHopSupport
                        | TopologyNoise::NoisySparseMultiHopSupport
                        | TopologyNoise::NoisySparseMultiHopContradicted
                        | TopologyNoise::NoisySparseMultiHopContradictedActiveHints
                ) {
                    for topic in 0..topic_count {
                        let next_topic = (topic + 1) % topic_count;
                        for duplicate in 0..duplicates {
                            let summary = topic_nodes[topic][0][duplicate];
                            let target_fact = 1 + duplicate % (FACTS_PER_TOPIC - 1);
                            let target_nodes = &topic_nodes[next_topic][target_fact];
                            let target = current_replacement(target_nodes, duplicate);
                            mutator
                                .create_edge(
                                    support_edge.clone(),
                                    summary,
                                    target,
                                    PropertyMap::new(),
                                )
                                .expect("bench noisy sparse support edge inserts");
                        }
                    }
                }
                for facts in &topic_nodes {
                    for (duplicate, summary) in facts[0].iter().enumerate() {
                        for (fact, evidence_nodes) in facts.iter().enumerate().skip(1) {
                            if !support_edge_included(topology, duplicate, fact) {
                                continue;
                            }
                            let evidence = evidence_nodes[duplicate % evidence_nodes.len()];
                            if matches!(
                                topology,
                                TopologyNoise::MultiHopSupport
                                    | TopologyNoise::NoisyMultiHopSupport
                                    | TopologyNoise::NoisySparseMultiHopSupport
                                    | TopologyNoise::NoisySparseMultiHopContradicted
                                    | TopologyNoise::NoisySparseMultiHopContradictedActiveHints
                            ) && fact >= SEED_K
                            {
                                let bridge = mutator
                                    .create_node(
                                        LabelSet::single(bridge_label.clone()),
                                        PropertyMap::new(),
                                    )
                                    .expect("bench support bridge inserts");
                                mutator
                                    .create_edge(
                                        support_edge.clone(),
                                        *summary,
                                        bridge,
                                        PropertyMap::new(),
                                    )
                                    .expect("bench summary-to-bridge support edge inserts");
                                mutator
                                    .create_edge(
                                        support_edge.clone(),
                                        bridge,
                                        evidence,
                                        PropertyMap::new(),
                                    )
                                    .expect("bench bridge-to-evidence support edge inserts");
                                if metadata.get(&evidence).is_some_and(|meta| meta.current) {
                                    mutator
                                        .create_edge(
                                            valid_edge.clone(),
                                            bridge,
                                            evidence,
                                            PropertyMap::new(),
                                        )
                                        .expect("bench bridge valid edge inserts");
                                } else {
                                    let replacement =
                                        current_replacement(evidence_nodes, duplicate);
                                    mutator
                                        .create_edge(
                                            superseded_by_edge.clone(),
                                            evidence,
                                            replacement,
                                            PropertyMap::new(),
                                        )
                                        .expect("bench bridge supersession edge inserts");
                                }
                                continue;
                            }
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
                if matches!(
                    topology,
                    TopologyNoise::ContradictedCurrentDuplicates
                        | TopologyNoise::NoisySparseMultiHopContradicted
                        | TopologyNoise::NoisySparseMultiHopContradictedActiveHints
                ) {
                    add_contradicted_current_duplicates(
                        &mut mutator,
                        &topic_nodes,
                        &metadata,
                        &contradicts_edge,
                    );
                }
                if topology == TopologyNoise::NoisySparseMultiHopContradictedActiveHints {
                    add_active_hint_edges(
                        &mut mutator,
                        &topic_nodes,
                        &recent_label,
                        &recent_edge,
                        &depends_edge,
                    );
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
        let graph_current_nodes =
            materialized_current_nodes(&graph, metadata.keys().copied(), &superseded_by_edge);
        let graph_unresolved_current_nodes = materialized_unresolved_current_nodes(
            &graph,
            metadata.keys().copied(),
            &superseded_by_edge,
            &contradicts_edge,
        );
        let graph_unresolved_current_candidate_set = selene_graph::VectorCandidateSet::from_nodes(
            graph_unresolved_current_nodes.iter().copied(),
        );
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
            session_edge,
            valid_edge,
            superseded_by_edge,
            contradicts_edge,
            recent_edge,
            depends_edge,
            queries,
            metadata,
            graph_current_nodes,
            graph_unresolved_current_nodes,
            graph_unresolved_current_candidate_set,
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

fn support_edge_included(topology: TopologyNoise, duplicate: usize, fact: usize) -> bool {
    match topology {
        TopologyNoise::SparseSupport
        | TopologyNoise::NoisySparseSupport
        | TopologyNoise::NoisySparseMultiHopSupport
        | TopologyNoise::NoisySparseMultiHopContradicted
        | TopologyNoise::NoisySparseMultiHopContradictedActiveHints => {
            (fact - 1) % SEED_K == duplicate % SEED_K
        }
        TopologyNoise::Clean
        | TopologyNoise::CrossTopicSupportRing
        | TopologyNoise::MultiHopSupport
        | TopologyNoise::NoisyMultiHopSupport
        | TopologyNoise::ContradictedCurrentDuplicates => true,
    }
}

fn add_active_hint_edges(
    mutator: &mut selene_graph::Mutator<'_, '_>,
    topic_nodes: &[Vec<Vec<NodeId>>],
    recent_label: &DbString,
    recent_edge: &DbString,
    depends_edge: &DbString,
) {
    for facts in topic_nodes {
        let anchor = facts[0][facts[0].len() / 2];
        let recent_window = mutator
            .create_node(LabelSet::single(recent_label.clone()), PropertyMap::new())
            .expect("bench recent window insert succeeds");
        mutator
            .create_edge(
                recent_edge.clone(),
                anchor,
                recent_window,
                PropertyMap::new(),
            )
            .expect("bench anchor-to-recent edge inserts");
        for nodes in facts {
            for &node in nodes {
                if node == anchor {
                    continue;
                }
                mutator
                    .create_edge(recent_edge.clone(), node, recent_window, PropertyMap::new())
                    .expect("bench recent membership edge inserts");
            }
        }
        for nodes in facts {
            mutator
                .create_edge(depends_edge.clone(), anchor, nodes[0], PropertyMap::new())
                .expect("bench dependency edge inserts");
        }
    }
}

fn add_contradicted_current_duplicates(
    mutator: &mut selene_graph::Mutator<'_, '_>,
    topic_nodes: &[Vec<Vec<NodeId>>],
    metadata: &HashMap<NodeId, NodeMeta>,
    contradicts_edge: &DbString,
) {
    for facts in topic_nodes {
        for nodes in facts {
            let canonical = nodes[0];
            for &node in nodes.iter().skip(1) {
                if metadata.get(&node).is_some_and(|meta| meta.current) {
                    mutator
                        .create_edge(
                            contradicts_edge.clone(),
                            node,
                            canonical,
                            PropertyMap::new(),
                        )
                        .expect("bench contradiction edge inserts");
                }
            }
        }
    }
}

fn materialized_current_nodes<I>(
    graph: &SeleneGraph,
    nodes: I,
    superseded_by_edge: &DbString,
) -> HashSet<NodeId>
where
    I: IntoIterator<Item = NodeId>,
{
    nodes
        .into_iter()
        .filter(|node_id| {
            !graph
                .outgoing_edges(*node_id)
                .is_some_and(|edges| edges.iter().any(|edge| edge.label == *superseded_by_edge))
        })
        .collect()
}

fn materialized_unresolved_current_nodes<I>(
    graph: &SeleneGraph,
    nodes: I,
    superseded_by_edge: &DbString,
    contradicts_edge: &DbString,
) -> HashSet<NodeId>
where
    I: IntoIterator<Item = NodeId>,
{
    nodes
        .into_iter()
        .filter(|node_id| {
            !graph.outgoing_edges(*node_id).is_some_and(|edges| {
                edges.iter().any(|edge| {
                    edge.label == *superseded_by_edge || edge.label == *contradicts_edge
                })
            })
        })
        .collect()
}
