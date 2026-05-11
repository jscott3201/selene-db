//! Articulation points and bridges via iterative Hopcroft-Tarjan DFS.
//!
//! Both algorithms share a single `lowlink_pass` over the **undirected** view
//! of the projection (union of out- and in-neighbors per node, deduped and
//! sorted ASC by row per spec 16 §E03). The pass returns both result sets so
//! callers pay for the DFS once. Per spec 16 §E11 the DFS is iterative with an
//! explicit work-stack — no recursion.

use std::collections::{HashMap, HashSet};

use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::SENTINEL;

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
    let Some(max_row) = proj.max_row() else {
        return (Vec::new(), Vec::new());
    };
    let size = max_row as usize + 1;
    let mut state = BiconnState::with_capacity(size);

    for nid in proj.iter_nodes() {
        let row = node_row(nid);
        if state.disc[row as usize] == SENTINEL {
            biconn_dfs(&mut state, row, proj);
        }
    }

    // Materialize results in canonical order.
    let mut ap: Vec<NodeId> = state
        .ap
        .iter()
        .map(|&row| NodeId::new(u64::from(row) + 1))
        .collect();
    ap.sort_by_key(|nid| nid.get());

    let mut bridges: Vec<(NodeId, NodeId)> = state
        .bridges
        .iter()
        .map(|&(a, b)| (NodeId::new(u64::from(a) + 1), NodeId::new(u64::from(b) + 1)))
        .collect();
    bridges.sort_by_key(|&(s, t)| (s.get(), t.get()));

    (ap, bridges)
}

/// DFS state for articulation + bridges (`disc` / `low` / `parent` / `ap` /
/// `bridges`).
struct BiconnState {
    timer: u32,
    disc: Vec<u32>,
    low: Vec<u32>,
    /// `parent[row]` = parent row in the DFS tree, or `SENTINEL` for DFS roots.
    parent: Vec<u32>,
    ap: HashSet<u32>,
    bridges: Vec<(u32, u32)>,
}

impl BiconnState {
    fn with_capacity(size: usize) -> Self {
        Self {
            timer: 0,
            disc: vec![SENTINEL; size],
            low: vec![SENTINEL; size],
            parent: vec![SENTINEL; size],
            ap: HashSet::new(),
            bridges: Vec::new(),
        }
    }
}

/// Iterative biconnectivity DFS (donor pattern,
/// `aether-db-algorithms/src/structural.rs:486-554`). Frame is
/// `(row, neighbor_index, dfs_children_count)`.
fn biconn_dfs(state: &mut BiconnState, start: u32, proj: &GraphProjection) {
    let mut call_stack: Vec<(u32, usize, u32)> = Vec::new();
    // Per-DFS undirected neighbor cache: row → sorted-deduped neighbor rows.
    let mut neighbors_cache: HashMap<u32, Vec<u32>> = HashMap::new();

    let si = start as usize;
    state.disc[si] = state.timer;
    state.low[si] = state.timer;
    state.timer += 1;
    call_stack.push((start, 0, 0));

    while let Some(&mut (u, ref mut ni, ref mut children)) = call_stack.last_mut() {
        let neighbors = neighbors_cache.entry(u).or_insert_with(|| {
            // Build the undirected neighbor view: out ∪ in, deduped via HashSet,
            // then sorted ASC by row for E03/E12 determinism.
            let nid = NodeId::new(u64::from(u) + 1);
            let mut set: HashSet<u32> = HashSet::new();
            for nb in proj.out_neighbors(nid) {
                set.insert(node_row(nb.node_id));
            }
            for nb in proj.in_neighbors(nid) {
                set.insert(node_row(nb.node_id));
            }
            let mut v: Vec<u32> = set.into_iter().collect();
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
                call_stack.push((v, 0, 0));
            } else if state.parent[ui] != v {
                // Back-edge (not to the immediate DFS parent): update lowlink.
                state.low[ui] = state.low[ui].min(state.disc[vi]);
            }
        } else {
            // Frame `u` complete; propagate lowlink upward and detect cut nodes
            // / bridges relative to the parent.
            let finished_u = u;
            let finished_children = *children;
            call_stack.pop();
            let fi = finished_u as usize;

            if let Some(&mut (parent, _, _)) = call_stack.last_mut() {
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

/// Map a `NodeId` (1-based per selene-graph) to its row index (0-based).
#[inline]
fn node_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
