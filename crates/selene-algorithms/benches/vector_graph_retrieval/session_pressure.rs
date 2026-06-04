//! Task/session graph filter rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::NodeId;

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, SEED_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality, TopologyNoise};

const SESSION_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::NoisyWcc,
    SessionStrategy::LabelPropagation,
    SessionStrategy::GraphSessionFilter,
    SessionStrategy::GraphSessionCurrentFilter,
    SessionStrategy::GraphSessionUnsupersededFilter,
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpand,
    SessionStrategy::GraphScopeFilter,
    SessionStrategy::GraphScopeCurrentFilter,
    SessionStrategy::GraphScopeUnsupersededFilter,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpand,
    SessionStrategy::TopicFilter,
];

#[derive(Clone, Copy, Debug)]
enum SessionStrategy {
    NoisyWcc,
    LabelPropagation,
    GraphSessionFilter,
    GraphSessionCurrentFilter,
    GraphSessionUnsupersededFilter,
    GraphSessionMaterializedCurrentFilter,
    GraphSessionProvenanceExpand,
    GraphScopeFilter,
    GraphScopeCurrentFilter,
    GraphScopeUnsupersededFilter,
    GraphScopeMaterializedCurrentFilter,
    GraphScopeProvenanceExpand,
    TopicFilter,
}

impl SessionStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::NoisyWcc => "noisy_wcc",
            Self::LabelPropagation => "label_propagation",
            Self::GraphSessionFilter => "graph_session_filter",
            Self::GraphSessionCurrentFilter => "graph_session_current_filter",
            Self::GraphSessionUnsupersededFilter => "graph_session_unsuperseded_filter",
            Self::GraphSessionMaterializedCurrentFilter => {
                "graph_session_materialized_current_filter"
            }
            Self::GraphSessionProvenanceExpand => "graph_session_provenance_expand",
            Self::GraphScopeFilter => "graph_scope_filter",
            Self::GraphScopeCurrentFilter => "graph_scope_current_filter",
            Self::GraphScopeUnsupersededFilter => "graph_scope_unsuperseded_filter",
            Self::GraphScopeMaterializedCurrentFilter => "graph_scope_materialized_current_filter",
            Self::GraphScopeProvenanceExpand => "graph_scope_provenance_expand",
            Self::TopicFilter => "topic_filter",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_session_filter_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_community_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in SESSION_STRATEGIES {
            let avg_candidates = fixture.average_session_candidates(strategy);
            let quality = fixture.session_quality(strategy);
            group.throughput(Throughput::Elements(
                (fixture.query_count() * avg_candidates) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    strategy.name(),
                    format!(
                        "{}_q{}_c{}_covbp{}_curbp{}_precbp{}",
                        scale_label(fixture.scale()),
                        fixture.query_count(),
                        avg_candidates,
                        basis_points(quality.coverage, fixture.query_count() * FACTS_PER_TOPIC),
                        basis_points(
                            quality.current_coverage,
                            fixture.query_count() * FACTS_PER_TOPIC
                        ),
                        basis_points(quality.precision, fixture.query_count() * RESULT_K),
                    ),
                ),
                |b| {
                    b.iter(|| {
                        black_box(fixture.session_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn session_quality(&self, strategy: SessionStrategy) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.session_query_quality(query, strategy))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn session_total_coverage(&self, strategy: SessionStrategy) -> usize {
        self.session_quality(strategy).coverage
    }

    fn session_query_quality(&self, query: &Query, strategy: SessionStrategy) -> RetrievalQuality {
        let selected = self.select_session_candidates(query, strategy);
        self.selected_quality(query, selected)
    }

    fn select_session_candidates(&self, query: &Query, strategy: SessionStrategy) -> Vec<NodeId> {
        let candidates = self.session_candidates(query, strategy);
        if matches!(
            strategy,
            SessionStrategy::GraphSessionProvenanceExpand
                | SessionStrategy::GraphScopeProvenanceExpand
        ) {
            return self.select_provenance_expansion(query, candidates);
        }
        let hits = self.score_candidate_ids(query, candidates);
        self.select_from_candidates(query, hits, true, false, true)
    }

    fn session_candidates(&self, query: &Query, strategy: SessionStrategy) -> Vec<NodeId> {
        match strategy {
            SessionStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .cloned()
                .unwrap_or_default(),
            SessionStrategy::LabelPropagation => self
                .label_by_node
                .get(&query.anchor)
                .and_then(|community| self.label_candidates.get(community))
                .cloned()
                .unwrap_or_default(),
            SessionStrategy::GraphSessionFilter => self.graph_session_candidates(query),
            SessionStrategy::GraphSessionCurrentFilter => {
                self.current_candidates(self.graph_session_candidates(query))
            }
            SessionStrategy::GraphSessionUnsupersededFilter => {
                self.unsuperseded_candidates(self.graph_session_candidates(query))
            }
            SessionStrategy::GraphSessionMaterializedCurrentFilter => {
                self.materialized_current_candidates(self.graph_session_candidates(query))
            }
            SessionStrategy::GraphSessionProvenanceExpand => self.provenance_root_candidates(
                self.materialized_current_candidates(self.graph_session_candidates(query)),
            ),
            SessionStrategy::GraphScopeFilter => self.graph_session_scope_candidates(query),
            SessionStrategy::GraphScopeCurrentFilter => {
                self.current_candidates(self.graph_session_scope_candidates(query))
            }
            SessionStrategy::GraphScopeUnsupersededFilter => {
                self.unsuperseded_candidates(self.graph_session_scope_candidates(query))
            }
            SessionStrategy::GraphScopeMaterializedCurrentFilter => {
                self.materialized_current_candidates(self.graph_session_scope_candidates(query))
            }
            SessionStrategy::GraphScopeProvenanceExpand => self.provenance_root_candidates(
                self.materialized_current_candidates(self.graph_session_scope_candidates(query)),
            ),
            SessionStrategy::TopicFilter => self
                .topic_candidates
                .get(query.topic)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn graph_session_candidates(&self, query: &Query) -> Vec<NodeId> {
        let mut candidates = Vec::new();
        if let Some(anchor_edges) = self.graph.outgoing_edges(query.anchor) {
            for session_edge in anchor_edges
                .iter()
                .filter(|edge| edge.label == self.session_edge)
            {
                if let Some(session_members) = self.graph.incoming_edges(session_edge.neighbor) {
                    candidates.extend(
                        session_members
                            .iter()
                            .filter(|edge| edge.label == self.session_edge)
                            .map(|edge| edge.neighbor),
                    );
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    fn current_candidates(&self, candidates: Vec<NodeId>) -> Vec<NodeId> {
        candidates
            .into_iter()
            .filter(|node_id| self.is_current(*node_id))
            .collect()
    }

    fn materialized_current_candidates(&self, candidates: Vec<NodeId>) -> Vec<NodeId> {
        candidates
            .into_iter()
            .filter(|node_id| self.graph_current_nodes.contains(node_id))
            .collect()
    }

    fn unsuperseded_candidates(&self, candidates: Vec<NodeId>) -> Vec<NodeId> {
        candidates
            .into_iter()
            .filter(|node_id| !self.has_superseded_by_edge(*node_id))
            .collect()
    }

    fn provenance_root_candidates(&self, candidates: Vec<NodeId>) -> Vec<NodeId> {
        candidates
            .into_iter()
            .filter(|node_id| self.has_support_edge(*node_id))
            .collect()
    }

    fn has_support_edge(&self, node_id: NodeId) -> bool {
        self.graph
            .outgoing_edges(node_id)
            .is_some_and(|edges| edges.iter().any(|edge| edge.label == self.support_edge))
    }

    fn has_superseded_by_edge(&self, node_id: NodeId) -> bool {
        self.graph.outgoing_edges(node_id).is_some_and(|edges| {
            edges
                .iter()
                .any(|edge| edge.label == self.superseded_by_edge)
        })
    }

    fn select_provenance_expansion(
        &self,
        query: &Query,
        root_candidates: Vec<NodeId>,
    ) -> Vec<NodeId> {
        let mut roots = self.score_candidate_ids(query, root_candidates);
        roots.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.node_id.cmp(&right.node_id))
        });

        let mut expanded = Vec::new();
        for root in roots.into_iter().take(SEED_K) {
            expanded.push(root.node_id);
            if let Some(edges) = self.graph.outgoing_edges(root.node_id) {
                expanded.extend(
                    edges
                        .iter()
                        .filter(|edge| edge.label == self.support_edge)
                        .map(|edge| {
                            self.superseded_replacement(edge.neighbor)
                                .unwrap_or(edge.neighbor)
                        }),
                );
            }
        }
        self.select_current_diverse_nodes(query, expanded)
    }

    fn select_current_diverse_nodes(&self, query: &Query, candidates: Vec<NodeId>) -> Vec<NodeId> {
        let mut selected = Vec::with_capacity(RESULT_K);
        let mut seen_facts = std::collections::HashSet::new();
        let mut deferred = Vec::new();
        for node_id in candidates {
            if !self.graph_current_nodes.contains(&node_id) {
                continue;
            }
            let fact_key = self
                .metadata
                .get(&node_id)
                .filter(|meta| meta.topic == query.topic)
                .map(|meta| meta.fact);
            if let Some(fact) = fact_key
                && seen_facts.insert(fact)
            {
                selected.push(node_id);
            } else {
                deferred.push(node_id);
            }
            if selected.len() == RESULT_K {
                return selected;
            }
        }
        selected.extend(deferred.into_iter().take(RESULT_K - selected.len()));
        selected
    }

    fn graph_session_scope_candidates(&self, query: &Query) -> Vec<NodeId> {
        let mut candidates = Vec::new();
        if let Some(anchor_edges) = self.graph.outgoing_edges(query.anchor) {
            for scope_edge in anchor_edges
                .iter()
                .filter(|edge| edge.label == self.scope_edge)
            {
                if let Some(scope_members) = self.graph.incoming_edges(scope_edge.neighbor) {
                    candidates.extend(
                        scope_members
                            .iter()
                            .filter(|edge| edge.label == self.scope_edge)
                            .map(|edge| edge.neighbor),
                    );
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    fn session_candidate_count(&self, query: &Query, strategy: SessionStrategy) -> usize {
        self.session_candidates(query, strategy).len()
    }

    fn average_session_candidates(&self, strategy: SessionStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.session_candidate_count(query, strategy))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }
}
