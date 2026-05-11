//! Articulation points and bridges via iterative Hopcroft-Tarjan DFS.
//!
//! Both algorithms share a single `lowlink_pass` over the **undirected** view
//! of the projection (union of out- and in-neighbors per node, **with
//! multiplicity preserved** and sorted ASC by dense index per spec 16 §E03).
//! The pass returns both result sets so callers pay for the DFS once. Per
//! spec 16 §E11 the DFS is iterative with an explicit work-stack — no
//! recursion.
//!
//! ## Why multiplicity is preserved
//!
//! Codex P1 review on BRIEF-52 PR #58 caught that `HashSet`-based dedupe
//! collapses parallel edges between the same endpoints: two `friend` edges
//! between `n1` and `n2` would be seen as a single edge by the DFS, and
//! removing one would (incorrectly) be reported as disconnecting the graph.
//! The graph layer allows parallel edges (no uniqueness check in
//! `create_edge`), so the algorithm MUST preserve multiplicity. The
//! "parent skip" rule then consumes only the **first** occurrence of the
//! parent in `u`'s neighbor list; subsequent occurrences are legitimate
//! parallel back-edges that update `low[u]`.
//!
//! ## Why state arrays are sized by live count
//!
//! Same review caught that sizing arrays by `max_row + 1` ties memory to the
//! largest historical NodeId rather than the live-node count. State is now
//! indexed by dense indices via [`RowIndex`]; sparse rows are translated at
//! the projection boundary.

use std::collections::HashMap;

use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::{RowIndex, SENTINEL};

/// Articulation points (cut vertices) under the projection's undirected view.
///
/// Returns `NodeId`s sorted ASC. Empty projection → empty `Vec` per spec 16
/// §E09.
#[must_use]
pub fn articulation_points(proj: &GraphProjection) -> Vec<NodeId> {
    let (ap, _) = lowlink_pass(proj);
    ap
}

/// Bridges (cut edges) under the projection's undirected view.
///
/// Returns endpoint pairs `(source, target)` where `source.get() < target.get()`
/// (canonicalized at insert per donor `aether-db-algorithms/src/structural.rs:545`),
/// and the outer `Vec` is sorted ASC by `(source, target)`. Empty projection →
/// empty `Vec` per spec 16 §E09.
#[must_use]
pub fn bridges(proj: &GraphProjection) -> Vec<(NodeId, NodeId)> {
    let (_, bridges) = lowlink_pass(proj);
    bridges
}

/// Shared DFS computing both articulation points and bridges in a single pass.
///
/// Returns `(articulation_points_sorted_asc, bridges_sorted_asc)`. Both are
/// canonicalized per E12 for deterministic output.
fn lowlink_pass(proj: &GraphProjection) -> (Vec<NodeId>, Vec<(NodeId, NodeId)>) {
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut state = BiconnState::with_capacity(idx.len());

    for d in 0..idx.len() as u32 {
        if state.disc[d as usize] == SENTINEL {
            biconn_dfs(&mut state, d, proj, &idx);
        }
    }

    // Materialize results in canonical order using dense → NodeId conversion.
    let mut ap: Vec<NodeId> = state.ap.iter().map(|&d| idx.node_id_of(d)).collect();
    ap.sort_by_key(|nid| nid.get());

    let mut bridges: Vec<(NodeId, NodeId)> = state
        .bridges
        .iter()
        .map(|&(a, b)| (idx.node_id_of(a), idx.node_id_of(b)))
        .collect();
    bridges.sort_by_key(|&(s, t)| (s.get(), t.get()));

    (ap, bridges)
}

/// DFS state for articulation + bridges (`disc` / `low` / `parent` / `ap` /
/// `bridges`). All indexed by dense indices in `0..idx.len()`.
struct BiconnState {
    timer: u32,
    disc: Vec<u32>,
    low: Vec<u32>,
    /// `parent[dense]` = parent dense index in the DFS tree, or `SENTINEL`
    /// for DFS roots.
    parent: Vec<u32>,
    ap: std::collections::HashSet<u32>,
    bridges: Vec<(u32, u32)>,
}

impl BiconnState {
    fn with_capacity(size: usize) -> Self {
        Self {
            timer: 0,
            disc: vec![SENTINEL; size],
            low: vec![SENTINEL; size],
            parent: vec![SENTINEL; size],
            ap: std::collections::HashSet::new(),
            bridges: Vec::new(),
        }
    }
}

/// Iterative biconnectivity DFS (donor pattern,
/// `aether-db-algorithms/src/structural.rs:486-554`). Frame is
/// `(dense, neighbor_index, dfs_children_count, parent_edge_consumed)`.
///
/// `parent_edge_consumed` tracks whether we've already skipped the one tree
/// edge from this frame back to its parent — subsequent occurrences of the
/// parent in the neighbor list are parallel back-edges and DO update `low`.
fn biconn_dfs(state: &mut BiconnState, start: u32, proj: &GraphProjection, idx: &RowIndex) {
    let mut call_stack: Vec<(u32, usize, u32, bool)> = Vec::new();
    // Per-DFS undirected neighbor cache: dense → sorted-with-multiplicity
    // neighbor dense indices. Multiplicity is preserved (no HashSet dedupe)
    // so parallel edges are visible to the lowlink rule.
    let mut neighbors_cache: HashMap<u32, Vec<u32>> = HashMap::new();

    state.disc[start as usize] = state.timer;
    state.low[start as usize] = state.timer;
    state.timer += 1;
    call_stack.push((start, 0, 0, false));

    while let Some(&mut (u, ref mut ni, ref mut children, ref mut parent_consumed)) =
        call_stack.last_mut()
    {
        let neighbors = neighbors_cache.entry(u).or_insert_with(|| {
            // Build the undirected neighbor view: out + in, preserving
            // multiplicity, then sorted ASC by dense index for E03/E12
            // determinism. Neighbors outside the projection scope are dropped.
            let nid = idx.node_id_of(u);
            let mut v: Vec<u32> = Vec::new();
            for nb in proj.out_neighbors(nid) {
                if let Some(d) = idx.dense_of(node_sparse_row(nb.node_id)) {
                    v.push(d);
                }
            }
            for nb in proj.in_neighbors(nid) {
                if let Some(d) = idx.dense_of(node_sparse_row(nb.node_id)) {
                    v.push(d);
                }
            }
            v.sort_unstable();
            v
        });

        if *ni < neighbors.len() {
            let v = neighbors[*ni];
            *ni += 1;
            let vi = v as usize;
            let ui = u as usize;

            if state.disc[vi] == SENTINEL {
                *children += 1;
                state.parent[vi] = u;
                state.disc[vi] = state.timer;
                state.low[vi] = state.timer;
                state.timer += 1;
                call_stack.push((v, 0, 0, false));
            } else if v == state.parent[ui] && !*parent_consumed {
                // First encounter of the parent edge: this is the tree edge
                // we descended from; skip lowlink update.
                *parent_consumed = true;
            } else {
                // Back-edge: either a true non-parent back-edge OR a parallel
                // back-edge to the parent. Update lowlink either way.
                state.low[ui] = state.low[ui].min(state.disc[vi]);
            }
        } else {
            // Frame `u` complete; propagate lowlink upward and detect cut
            // nodes / bridges relative to the parent.
            let finished_u = u;
            let finished_children = *children;
            call_stack.pop();
            let fi = finished_u as usize;

            if let Some(&mut (parent, _, _, _)) = call_stack.last_mut() {
                let pi = parent as usize;
                state.low[pi] = state.low[pi].min(state.low[fi]);

                // Why: parent[pi] == SENTINEL iff `pi` is a DFS root.
                // SENTINEL is the initial parent value; we only overwrite it
                // inside the unvisited-child branch above, which fires before
                // `pi` could become a root in any deeper DFS subtree.
                let is_root = state.parent[pi] == SENTINEL;
                if !is_root && state.low[fi] >= state.disc[pi] {
                    state.ap.insert(parent);
                }

                if state.low[fi] > state.disc[pi] {
                    // Canonicalize bridge endpoints at insert (donor:545).
                    state
                        .bridges
                        .push((parent.min(finished_u), parent.max(finished_u)));
                }
            }

            // Root with ≥ 2 DFS children is an articulation point.
            if state.parent[fi] == SENTINEL && finished_children > 1 {
                state.ap.insert(finished_u);
            }
        }
    }
}

/// Map a `NodeId` (1-based per selene-graph) to its sparse row index
/// (0-based). Used at the projection boundary to look up neighbor rows
/// before converting to dense indices via [`RowIndex`].
#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
