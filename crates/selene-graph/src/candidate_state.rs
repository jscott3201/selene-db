//! Maintained graph-derived candidate sets.
//!
//! This module owns small, policy-neutral maintained node sets for graph/vector
//! retrieval. A set can require a node label and can exclude nodes that have at
//! least one outgoing or incoming edge with configured labels. That is enough to
//! model active/current/unresolved memory subsets without hard-coding those
//! application labels into the engine.

use std::collections::{BTreeMap, BTreeSet};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use selene_core::{Change, EdgeId, IStr, LabelSet, NodeId};

use crate::index_provider::{
    IndexProvider, ProviderError, ProviderTag, SubTag, VectorCandidateStateInfo,
};
use crate::store::RowIndex;
use crate::{SeleneGraph, VectorCandidateSet};

/// Provider tag for maintained graph candidate-state sections.
pub const CANDIDATE_STATE_PROVIDER_TAG: [u8; 4] = *b"CSET";

/// Provider-owned snapshot section for maintained candidate-state data.
pub const CANDIDATE_STATE_SUB: [u8; 4] = *b"STAT";

const SNAPSHOT_VERSION: u8 = 1;
const SUB_TAGS: &[SubTag] = &[SubTag(CANDIDATE_STATE_SUB)];

/// Declarative rule for one maintained candidate set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CandidateStateSpec {
    /// Stable set name used by callers to retrieve candidates.
    pub name: IStr,
    /// Optional node label required for membership.
    pub required_label: Option<IStr>,
    /// Outgoing edge labels required on the source node.
    pub require_outgoing: Vec<IStr>,
    /// Incoming edge labels required on the target node.
    pub require_incoming: Vec<IStr>,
    /// Outgoing edge labels that disqualify the source node.
    pub exclude_outgoing: Vec<IStr>,
    /// Incoming edge labels that disqualify the target node.
    pub exclude_incoming: Vec<IStr>,
}

impl CandidateStateSpec {
    /// Construct an unconstrained named candidate set.
    #[must_use]
    pub fn new(name: IStr) -> Self {
        Self {
            name,
            required_label: None,
            require_outgoing: Vec::new(),
            require_incoming: Vec::new(),
            exclude_outgoing: Vec::new(),
            exclude_incoming: Vec::new(),
        }
    }

    /// Require `label` for candidate membership.
    #[must_use]
    pub fn require_label(mut self, label: IStr) -> Self {
        self.required_label = Some(label);
        self
    }

    /// Require an outgoing edge carrying `label`.
    #[must_use]
    pub fn require_outgoing(mut self, label: IStr) -> Self {
        insert_sorted_unique(&mut self.require_outgoing, label);
        self
    }

    /// Require an incoming edge carrying `label`.
    #[must_use]
    pub fn require_incoming(mut self, label: IStr) -> Self {
        insert_sorted_unique(&mut self.require_incoming, label);
        self
    }

    /// Exclude nodes with an outgoing edge carrying `label`.
    #[must_use]
    pub fn exclude_outgoing(mut self, label: IStr) -> Self {
        insert_sorted_unique(&mut self.exclude_outgoing, label);
        self
    }

    /// Exclude nodes with an incoming edge carrying `label`.
    #[must_use]
    pub fn exclude_incoming(mut self, label: IStr) -> Self {
        insert_sorted_unique(&mut self.exclude_incoming, label);
        self
    }
}

/// First-party provider maintaining named graph-derived candidate sets.
pub struct MaintainedCandidateStateProvider {
    specs: Vec<CandidateStateSpec>,
    state: Mutex<CandidateState>,
}

impl MaintainedCandidateStateProvider {
    /// Construct an empty provider for `specs`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when two specs use the same name.
    pub fn new(specs: impl IntoIterator<Item = CandidateStateSpec>) -> Result<Self, ProviderError> {
        let mut specs = specs.into_iter().collect::<Vec<_>>();
        for spec in &mut specs {
            canonicalize_labels(&mut spec.require_outgoing);
            canonicalize_labels(&mut spec.require_incoming);
            canonicalize_labels(&mut spec.exclude_outgoing);
            canonicalize_labels(&mut spec.exclude_incoming);
        }
        validate_unique_specs(&specs)?;
        Ok(Self {
            state: Mutex::new(CandidateState::new(&specs)),
            specs,
        })
    }

    /// Construct a provider and initialize it from a graph snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when specs are invalid or the graph snapshot is
    /// internally inconsistent.
    pub fn from_graph(
        specs: impl IntoIterator<Item = CandidateStateSpec>,
        graph: &SeleneGraph,
    ) -> Result<Self, ProviderError> {
        let provider = Self::new(specs)?;
        provider.rebuild_from_graph(graph)?;
        Ok(provider)
    }

    /// Rebuild all maintained state from `graph`.
    ///
    /// This is the safe attachment path when a provider is registered against an
    /// already-populated graph instead of observing mutations from graph birth.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if live row-to-id mappings are inconsistent.
    pub fn rebuild_from_graph(&self, graph: &SeleneGraph) -> Result<(), ProviderError> {
        let mut rebuilt = CandidateState::new(&self.specs);
        for row in graph.live_nodes() {
            let row = RowIndex::new(row);
            let id = graph.node_id_for_row(row).ok_or_else(|| {
                inconsistent(format!("live node row {} has no external id", row.get()))
            })?;
            let labels = graph
                .node_labels(id)
                .ok_or_else(|| inconsistent(format!("live node {id} has no label column entry")))?;
            rebuilt.node_labels.insert(id, labels.clone());
        }
        for row in graph.live_edges() {
            let row = RowIndex::new(row);
            let id = graph.edge_id_for_row(row).ok_or_else(|| {
                inconsistent(format!("live edge row {} has no external id", row.get()))
            })?;
            let label = graph
                .edge_label(id)
                .ok_or_else(|| inconsistent(format!("live edge {id} has no label")))?;
            if !watches_label(&self.specs, label) {
                continue;
            }
            let (source, target) = graph
                .edge_endpoints(id)
                .ok_or_else(|| inconsistent(format!("live edge {id} has no endpoints")))?;
            rebuilt.edges.insert(
                id,
                TrackedEdge {
                    label: label.clone(),
                    source,
                    target,
                },
            );
        }
        rebuilt.rebuild_derived(&self.specs);
        rebuilt.generation = graph.meta.generation;
        *self.state.lock() = rebuilt;
        Ok(())
    }

    /// Return the configured spec named `name`.
    #[must_use]
    pub fn spec(&self, name: &IStr) -> Option<&CandidateStateSpec> {
        self.specs.iter().find(|spec| &spec.name == name)
    }

    /// Return the current candidate set for `name`.
    #[must_use]
    pub fn candidate_set(&self, name: &IStr) -> Option<VectorCandidateSet> {
        let state = self.state.lock();
        state.members.get(name).map(|members| {
            VectorCandidateSet::from_canonical_nodes(members.iter().copied().collect())
        })
    }

    /// Return the provider generation watermark.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.state.lock().generation
    }

    /// Return the current candidate set for `name` if it matches `generation`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when this provider has not applied every
    /// mutation through `generation`.
    pub fn candidate_set_at_generation(
        &self,
        name: &IStr,
        generation: u64,
    ) -> Result<Option<VectorCandidateSet>, ProviderError> {
        let state = self.state.lock();
        if state.generation != generation {
            return Err(inconsistent(format!(
                "candidate-state generation {} does not match graph generation {generation}",
                state.generation
            )));
        }
        Ok(state.members.get(name).map(|members| {
            VectorCandidateSet::from_canonical_nodes(members.iter().copied().collect())
        }))
    }

    /// Return generation-checked metadata for every configured candidate set.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when this provider has not applied every
    /// mutation through `generation`.
    pub fn candidate_state_infos_at_generation(
        &self,
        generation: u64,
    ) -> Result<Vec<VectorCandidateStateInfo>, ProviderError> {
        let state = self.state.lock();
        if state.generation != generation {
            return Err(inconsistent(format!(
                "candidate-state generation {} does not match graph generation {generation}",
                state.generation
            )));
        }
        Ok(self
            .specs
            .iter()
            .map(|spec| VectorCandidateStateInfo {
                name: spec.name.clone(),
                generation,
                candidate_count: state.members.get(&spec.name).map_or(0, BTreeSet::len),
                required_label: spec.required_label.clone(),
                require_outgoing: spec.require_outgoing.clone(),
                require_incoming: spec.require_incoming.clone(),
                exclude_outgoing: spec.exclude_outgoing.clone(),
                exclude_incoming: spec.exclude_incoming.clone(),
            })
            .collect())
    }

    /// Return true when `node` is currently a member of the named set.
    #[must_use]
    pub fn contains(&self, name: &IStr, node: NodeId) -> bool {
        self.state
            .lock()
            .members
            .get(name)
            .is_some_and(|members| members.contains(&node))
    }
}

impl IndexProvider for MaintainedCandidateStateProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(CANDIDATE_STATE_PROVIDER_TAG)
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        ensure_state_subtag(sub_tag)?;
        let snapshot: CandidateStateSnapshot = postcard::from_bytes(bytes).map_err(|error| {
            invalid_payload(format!("CSET/STAT postcard decode failed: {error}"))
        })?;
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(invalid_payload(format!(
                "unsupported CSET/STAT version {}",
                snapshot.version
            )));
        }
        if snapshot.specs != self.specs {
            return Err(invalid_payload(
                "CSET/STAT specs differ from provider configuration".to_owned(),
            ));
        }
        let mut state = CandidateState::new(&self.specs);
        state.generation = snapshot.generation;
        for (id, labels) in snapshot.node_labels {
            if state.node_labels.insert(id, labels).is_some() {
                return Err(invalid_payload(format!(
                    "duplicate node id {id} in CSET/STAT"
                )));
            }
        }
        for (id, edge) in snapshot.edges {
            if !watches_label(&self.specs, &edge.label) {
                return Err(invalid_payload(format!(
                    "unwatched edge label {} in CSET/STAT",
                    edge.label.as_str()
                )));
            }
            if !state.node_labels.contains_key(&edge.source)
                || !state.node_labels.contains_key(&edge.target)
            {
                return Err(invalid_payload(format!(
                    "tracked edge {id} references missing endpoint in CSET/STAT"
                )));
            }
            if state.edges.insert(id, edge).is_some() {
                return Err(invalid_payload(format!(
                    "duplicate edge id {id} in CSET/STAT"
                )));
            }
        }
        state.rebuild_derived(&self.specs);
        *self.state.lock() = state;
        Ok(())
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        ensure_state_subtag(sub_tag)?;
        let state = self.state.lock();
        let snapshot = CandidateStateSnapshot {
            version: SNAPSHOT_VERSION,
            generation: state.generation,
            specs: self.specs.clone(),
            node_labels: state
                .node_labels
                .iter()
                .map(|(id, labels)| (*id, labels.clone()))
                .collect(),
            edges: state
                .edges
                .iter()
                .map(|(id, edge)| (*id, edge.clone()))
                .collect(),
        };
        postcard::to_stdvec(&snapshot).map_err(|error| ProviderError::SerializationFailed {
            reason: format!("CSET/STAT postcard encode failed: {error}"),
        })
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.state.lock().apply_change(&self.specs, change)
    }

    fn handles_change_batches(&self) -> bool {
        true
    }

    fn on_changes(&self, changes: &[Change]) -> Result<(), ProviderError> {
        let mut state = self.state.lock();
        for change in changes {
            state.apply_change(&self.specs, change)?;
        }
        Ok(())
    }

    fn rebuild_from_graph(&self, graph: &SeleneGraph) -> Result<(), ProviderError> {
        MaintainedCandidateStateProvider::rebuild_from_graph(self, graph)
    }

    fn on_commit_applied(&self, generation: u64) -> Result<(), ProviderError> {
        self.state.lock().generation = generation;
        Ok(())
    }

    fn vector_candidate_set(
        &self,
        name: &IStr,
        generation: u64,
    ) -> Result<Option<VectorCandidateSet>, ProviderError> {
        self.candidate_set_at_generation(name, generation)
    }

    fn vector_candidate_state_infos(
        &self,
        generation: u64,
    ) -> Result<Vec<VectorCandidateStateInfo>, ProviderError> {
        self.candidate_state_infos_at_generation(generation)
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        SUB_TAGS
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TrackedEdge {
    label: IStr,
    source: NodeId,
    target: NodeId,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CandidateStateSnapshot {
    version: u8,
    generation: u64,
    specs: Vec<CandidateStateSpec>,
    node_labels: Vec<(NodeId, LabelSet)>,
    edges: Vec<(EdgeId, TrackedEdge)>,
}

#[derive(Clone, Debug)]
struct CandidateState {
    generation: u64,
    node_labels: BTreeMap<NodeId, LabelSet>,
    edges: BTreeMap<EdgeId, TrackedEdge>,
    outgoing_counts: BTreeMap<(NodeId, IStr), usize>,
    incoming_counts: BTreeMap<(NodeId, IStr), usize>,
    members: BTreeMap<IStr, BTreeSet<NodeId>>,
}

impl CandidateState {
    fn new(specs: &[CandidateStateSpec]) -> Self {
        Self {
            generation: 0,
            node_labels: BTreeMap::new(),
            edges: BTreeMap::new(),
            outgoing_counts: BTreeMap::new(),
            incoming_counts: BTreeMap::new(),
            members: empty_members(specs),
        }
    }

    fn apply_change(
        &mut self,
        specs: &[CandidateStateSpec],
        change: &Change,
    ) -> Result<(), ProviderError> {
        match change {
            Change::NodeCreated { id, labels, .. } => {
                if self.node_labels.insert(*id, labels.clone()).is_some() {
                    return Err(inconsistent(format!("duplicate node create for {id}")));
                }
                self.recompute_node(specs, *id);
            }
            Change::NodeUpdated {
                id, labels_diff, ..
            } => {
                let labels = self
                    .node_labels
                    .get_mut(id)
                    .ok_or_else(|| inconsistent(format!("label update for unknown node {id}")))?;
                for label in &labels_diff.removed {
                    labels.remove(label);
                }
                for label in &labels_diff.added {
                    labels.insert(label.clone());
                }
                self.recompute_node(specs, *id);
            }
            Change::NodeDeleted { id } => {
                self.node_labels.remove(id);
                self.remove_incident_edges(specs, *id);
                self.recompute_node(specs, *id);
            }
            Change::NodeLabelRemoved { id, label } => {
                let labels = self
                    .node_labels
                    .get_mut(id)
                    .ok_or_else(|| inconsistent(format!("label removal for unknown node {id}")))?;
                labels.remove(label);
                self.recompute_node(specs, *id);
            }
            Change::EdgeCreated {
                id,
                label,
                source,
                target,
                ..
            } => {
                if watches_label(specs, label) {
                    let edge = TrackedEdge {
                        label: label.clone(),
                        source: *source,
                        target: *target,
                    };
                    if self.edges.insert(*id, edge.clone()).is_some() {
                        return Err(inconsistent(format!("duplicate edge create for {id}")));
                    }
                    self.increment_edge(&edge);
                    self.recompute_node(specs, *source);
                    self.recompute_node(specs, *target);
                }
            }
            Change::EdgeDeleted { id } => {
                if let Some(edge) = self.edges.remove(id) {
                    self.decrement_edge(&edge);
                    self.recompute_node(specs, edge.source);
                    self.recompute_node(specs, edge.target);
                }
            }
            Change::GraphReset {} => {
                *self = Self::new(specs);
            }
            Change::NodesOfTypeTruncated { label } => {
                let removed = self
                    .node_labels
                    .iter()
                    .filter_map(|(id, labels)| labels.contains(label).then_some(*id))
                    .collect::<BTreeSet<_>>();
                if !removed.is_empty() {
                    self.node_labels.retain(|id, _| !removed.contains(id));
                    self.edges.retain(|_, edge| {
                        !removed.contains(&edge.source) && !removed.contains(&edge.target)
                    });
                    self.rebuild_derived(specs);
                }
            }
            Change::EdgesOfTypeTruncated { label } => {
                if watches_label(specs, label) {
                    self.edges.retain(|_, edge| edge.label != *label);
                    self.rebuild_derived(specs);
                }
            }
            Change::EdgeUpdated { .. }
            | Change::EdgePropertyRemoved { .. }
            | Change::NodePropertyRemoved { .. }
            | Change::SchemaChanged { .. } => {}
        }
        Ok(())
    }

    fn rebuild_derived(&mut self, specs: &[CandidateStateSpec]) {
        self.outgoing_counts.clear();
        self.incoming_counts.clear();
        self.members = empty_members(specs);
        for edge in self.edges.values().cloned().collect::<Vec<_>>() {
            self.increment_edge(&edge);
        }
        for id in self.node_labels.keys().copied().collect::<Vec<_>>() {
            self.recompute_node(specs, id);
        }
    }

    fn increment_edge(&mut self, edge: &TrackedEdge) {
        *self
            .outgoing_counts
            .entry((edge.source, edge.label.clone()))
            .or_insert(0) += 1;
        *self
            .incoming_counts
            .entry((edge.target, edge.label.clone()))
            .or_insert(0) += 1;
    }

    fn decrement_edge(&mut self, edge: &TrackedEdge) {
        decrement_count(&mut self.outgoing_counts, (edge.source, edge.label.clone()));
        decrement_count(&mut self.incoming_counts, (edge.target, edge.label.clone()));
    }

    fn remove_incident_edges(&mut self, specs: &[CandidateStateSpec], node: NodeId) {
        let incident = self
            .edges
            .iter()
            .filter_map(|(id, edge)| {
                (edge.source == node || edge.target == node).then_some((*id, edge.clone()))
            })
            .collect::<Vec<_>>();
        for (id, edge) in incident {
            self.edges.remove(&id);
            self.decrement_edge(&edge);
            if edge.source != node {
                self.recompute_node(specs, edge.source);
            }
            if edge.target != node {
                self.recompute_node(specs, edge.target);
            }
        }
    }

    fn recompute_node(&mut self, specs: &[CandidateStateSpec], node: NodeId) {
        let labels = self.node_labels.get(&node).cloned();
        for spec in specs {
            let include = labels.as_ref().is_some_and(|labels| {
                spec.required_label
                    .as_ref()
                    .is_none_or(|required| labels.contains(required))
                    && spec
                        .require_outgoing
                        .iter()
                        .all(|label| has_count(&self.outgoing_counts, node, label))
                    && spec
                        .require_incoming
                        .iter()
                        .all(|label| has_count(&self.incoming_counts, node, label))
                    && spec
                        .exclude_outgoing
                        .iter()
                        .all(|label| !has_count(&self.outgoing_counts, node, label))
                    && spec
                        .exclude_incoming
                        .iter()
                        .all(|label| !has_count(&self.incoming_counts, node, label))
            });
            let members = self.members.entry(spec.name.clone()).or_default();
            if include {
                members.insert(node);
            } else {
                members.remove(&node);
            }
        }
    }
}

fn validate_unique_specs(specs: &[CandidateStateSpec]) -> Result<(), ProviderError> {
    let mut seen = BTreeSet::new();
    for spec in specs {
        if !seen.insert(spec.name.clone()) {
            return Err(inconsistent(format!(
                "duplicate candidate-state spec name {}",
                spec.name.as_str()
            )));
        }
    }
    Ok(())
}

fn empty_members(specs: &[CandidateStateSpec]) -> BTreeMap<IStr, BTreeSet<NodeId>> {
    specs
        .iter()
        .map(|spec| (spec.name.clone(), BTreeSet::new()))
        .collect()
}

fn watches_label(specs: &[CandidateStateSpec], label: &IStr) -> bool {
    specs.iter().any(|spec| {
        spec.require_outgoing.binary_search(label).is_ok()
            || spec.require_incoming.binary_search(label).is_ok()
            || spec.exclude_outgoing.binary_search(label).is_ok()
            || spec.exclude_incoming.binary_search(label).is_ok()
    })
}

fn has_count(counts: &BTreeMap<(NodeId, IStr), usize>, node: NodeId, label: &IStr) -> bool {
    counts
        .get(&(node, label.clone()))
        .is_some_and(|count| *count > 0)
}

fn decrement_count(counts: &mut BTreeMap<(NodeId, IStr), usize>, key: (NodeId, IStr)) {
    if let Some(count) = counts.get_mut(&key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            counts.remove(&key);
        }
    }
}

fn insert_sorted_unique(labels: &mut Vec<IStr>, label: IStr) {
    match labels.binary_search(&label) {
        Ok(_) => {}
        Err(index) => labels.insert(index, label),
    }
}

fn canonicalize_labels(labels: &mut Vec<IStr>) {
    labels.sort_unstable();
    labels.dedup();
}

fn ensure_state_subtag(sub_tag: SubTag) -> Result<(), ProviderError> {
    if sub_tag == SubTag(CANDIDATE_STATE_SUB) {
        Ok(())
    } else {
        Err(invalid_payload(format!("unknown CSET sub-tag {sub_tag}")))
    }
}

fn invalid_payload(reason: String) -> ProviderError {
    ProviderError::InvalidPayload { reason }
}

fn inconsistent(reason: String) -> ProviderError {
    ProviderError::Inconsistent { reason }
}

#[cfg(test)]
#[path = "candidate_state/tests.rs"]
mod tests;
