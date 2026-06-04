//! Selection helpers for session/scope graph retrieval pressure benchmarks.

use selene_core::NodeId;

use super::super::support::{FACTS_PER_TOPIC, RESULT_K};
use super::super::{MemoryRetrievalFixture, Query, RetrievalQuality};
use super::{ADAPTIVE_PROVENANCE_PLANS, SessionStrategy};

impl MemoryRetrievalFixture {
    pub(super) fn session_quality(&self, strategy: SessionStrategy) -> RetrievalQuality {
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

    pub(super) fn session_total_coverage(&self, strategy: SessionStrategy) -> usize {
        self.session_quality(strategy).coverage
    }

    pub(super) fn average_session_candidates(&self, strategy: SessionStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.session_candidate_count(query, strategy))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }

    fn session_query_quality(&self, query: &Query, strategy: SessionStrategy) -> RetrievalQuality {
        let selected = self.select_session_candidates(query, strategy);
        self.selected_quality(query, selected)
    }

    fn select_session_candidates(&self, query: &Query, strategy: SessionStrategy) -> Vec<NodeId> {
        let candidates = self.session_candidates(query, strategy);
        if strategy.is_adaptive_provenance() {
            return self.select_adaptive_provenance_expansion(query, candidates);
        }
        if let Some((seed_roots, depth)) = strategy.provenance_plan() {
            return self.select_provenance_expansion(query, candidates, seed_roots, depth);
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
            SessionStrategy::GraphSessionUnresolvedCurrentFilter => {
                self.unresolved_current_candidates(self.graph_session_candidates(query))
            }
            SessionStrategy::GraphSessionProvenanceExpandK1
            | SessionStrategy::GraphSessionProvenanceExpand2HopK1
            | SessionStrategy::GraphSessionProvenanceExpand
            | SessionStrategy::GraphSessionProvenanceExpand2Hop
            | SessionStrategy::GraphSessionProvenanceExpandK8
            | SessionStrategy::GraphSessionProvenanceExpand2HopK8
            | SessionStrategy::GraphSessionProvenanceExpandK16
            | SessionStrategy::GraphSessionProvenanceExpand2HopK16
            | SessionStrategy::GraphSessionProvenanceAdaptiveQuality => self
                .provenance_root_candidates(
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
            SessionStrategy::GraphScopeUnresolvedCurrentFilter => {
                self.unresolved_current_candidates(self.graph_session_scope_candidates(query))
            }
            SessionStrategy::GraphScopeProvenanceExpandK1
            | SessionStrategy::GraphScopeProvenanceExpand2HopK1
            | SessionStrategy::GraphScopeProvenanceExpand
            | SessionStrategy::GraphScopeProvenanceExpand2Hop
            | SessionStrategy::GraphScopeProvenanceExpandK8
            | SessionStrategy::GraphScopeProvenanceExpand2HopK8
            | SessionStrategy::GraphScopeProvenanceExpandK16
            | SessionStrategy::GraphScopeProvenanceExpand2HopK16
            | SessionStrategy::GraphScopeProvenanceAdaptiveQuality => self
                .provenance_root_candidates(
                    self.materialized_current_candidates(
                        self.graph_session_scope_candidates(query),
                    ),
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

    fn unresolved_current_candidates(&self, candidates: Vec<NodeId>) -> Vec<NodeId> {
        candidates
            .into_iter()
            .filter(|node_id| {
                self.graph_current_nodes.contains(node_id) && !self.has_contradicts_edge(*node_id)
            })
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

    fn has_contradicts_edge(&self, node_id: NodeId) -> bool {
        self.graph
            .outgoing_edges(node_id)
            .is_some_and(|edges| edges.iter().any(|edge| edge.label == self.contradicts_edge))
    }

    fn select_provenance_expansion(
        &self,
        query: &Query,
        root_candidates: Vec<NodeId>,
        seed_roots: usize,
        depth: usize,
    ) -> Vec<NodeId> {
        let roots = self.sorted_provenance_roots(query, root_candidates);
        let expanded = self.expand_provenance_roots(&roots, seed_roots, depth);
        self.select_current_diverse_nodes(query, expanded)
    }

    fn select_adaptive_provenance_expansion(
        &self,
        query: &Query,
        root_candidates: Vec<NodeId>,
    ) -> Vec<NodeId> {
        let roots = self.sorted_provenance_roots(query, root_candidates);
        let mut best = Vec::new();
        let mut best_quality = RetrievalQuality::default();
        for &(seed_roots, depth) in ADAPTIVE_PROVENANCE_PLANS {
            let expanded = self.expand_provenance_roots(&roots, seed_roots, depth);
            let selected = self.select_current_diverse_nodes(query, expanded);
            let quality = self.selected_quality(query, selected.clone());
            if quality_is_full(quality) {
                return selected;
            }
            if quality_tuple(quality) > quality_tuple(best_quality) {
                best = selected;
                best_quality = quality;
            }
        }
        best
    }

    fn sorted_provenance_roots(&self, query: &Query, root_candidates: Vec<NodeId>) -> Vec<NodeId> {
        let mut roots = self.score_candidate_ids(query, root_candidates);
        roots.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        roots.into_iter().map(|root| root.node_id).collect()
    }

    fn expand_provenance_roots(
        &self,
        roots: &[NodeId],
        seed_roots: usize,
        depth: usize,
    ) -> Vec<NodeId> {
        let mut expanded = Vec::new();
        for &root in roots.iter().take(seed_roots) {
            expanded.push(root);
            let mut frontier = vec![root];
            for _ in 0..depth {
                let mut next_frontier = Vec::new();
                for source in frontier {
                    if let Some(edges) = self.graph.outgoing_edges(source) {
                        for edge in edges.iter().filter(|edge| edge.label == self.support_edge) {
                            expanded.push(
                                self.superseded_replacement(edge.neighbor)
                                    .unwrap_or(edge.neighbor),
                            );
                            next_frontier.push(edge.neighbor);
                        }
                    }
                }
                frontier = next_frontier;
            }
        }
        expanded
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
}

fn quality_tuple(quality: RetrievalQuality) -> (usize, usize, usize) {
    (
        quality.coverage,
        quality.current_coverage,
        quality.precision,
    )
}

fn quality_is_full(quality: RetrievalQuality) -> bool {
    quality.coverage == FACTS_PER_TOPIC
        && quality.current_coverage == FACTS_PER_TOPIC
        && quality.precision == RESULT_K
}
