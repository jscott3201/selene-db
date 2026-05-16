//! Connected components: WCC (union-find, undirected) and SCC (iterative Tarjan).
//!
//! Both surfaces follow donor `aether-db-algorithms/src/structural.rs:91-258`:
//! component IDs are remapped to the smallest NodeId in each component (per
//! spec 16 §E12), and Tarjan's DFS is an iterative work-stack walk (per E11).
//!
//! State arrays are sized by **live-node count** via [`RowIndex`], not by
//! `max_row + 1`. Why: a sparse projection (label filter, scope intersection,
//! or long-lived graph with tombstoned deletes per D11) can have a max row
//! orders of magnitude larger than the live count; sizing by max_row would tie
//! memory to the largest historical NodeId and risk OOM on small live
//! subgraphs (spec 16 §E11 amendment from PR #58 Codex review).

// Integer-keyed hot-path maps use FxHashMap to avoid SipHash overhead.
use rustc_hash::FxHashMap as HashMap;
use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::{RowIndex, SENTINEL};

// ---------------------------------------------------------------------------
// Weakly Connected Components (undirected)
// ---------------------------------------------------------------------------

/// Compute weakly connected components — `(NodeId, component_id)` pairs where
/// `component_id` is the smallest `NodeId` in each component.
///
/// Treats the projection as **undirected**: two nodes share a component iff a
/// path exists between them via the union of out-neighbors and in-neighbors.
///
/// Returns an empty `Vec` for an empty projection (spec 16 §E09). Pairs are
/// ordered ASC by `NodeId` (matches `iter_nodes()` per spec 16 §E03/§E12).
#[must_use]
pub fn wcc(proj: &GraphProjection) -> Vec<(NodeId, u64)> {
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Vec::new();
    }
    let mut uf = UnionFind::new(idx.len());
    union_all_edges(proj, &idx, &mut uf);

    // First pass: collect (NodeId, current_root_dense) for every projection
    // node.
    let mut pairs: Vec<(NodeId, u32)> = (0..idx.len() as u32)
        .map(|d| (idx.node_id_of(d), uf.find(d)))
        .collect();

    // Compute the minimum NodeId per current root to canonicalize component IDs
    // (donor `aether-db-algorithms/src/structural.rs:107-115`).
    let mut min_per_root: HashMap<u32, u64> = HashMap::default();
    for &(nid, root) in &pairs {
        min_per_root
            .entry(root)
            .and_modify(|existing| {
                if nid.get() < *existing {
                    *existing = nid.get();
                }
            })
            .or_insert(nid.get());
    }

    // Second pass: rewrite root → min NodeId.
    let mut result: Vec<(NodeId, u64)> = pairs
        .drain(..)
        .map(|(nid, root)| (nid, min_per_root[&root]))
        .collect();
    result.sort_by_key(|&(nid, _)| nid.get());
    result
}

/// Count weakly connected components without materializing per-node IDs.
///
/// Donor `aether-db-algorithms/src/structural.rs:121-136`: counts unique
/// union-find roots, avoiding the result-Vec allocation.
#[must_use]
pub fn wcc_count(proj: &GraphProjection) -> usize {
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return 0;
    }
    let mut uf = UnionFind::new(idx.len());
    union_all_edges(proj, &idx, &mut uf);

    let mut count = 0usize;
    for d in 0..idx.len() as u32 {
        if uf.find(d) == d {
            count += 1;
        }
    }
    count
}

/// Union every projection edge (directed → undirected) into `uf`. Edges whose
/// endpoints land outside the projection scope are skipped — matches the
/// in-degree counting guard in topological-sort.
fn union_all_edges(proj: &GraphProjection, idx: &RowIndex, uf: &mut UnionFind) {
    for d in 0..idx.len() as u32 {
        let nid = idx.node_id_of(d);
        for nb in proj.out_neighbors(nid) {
            if let Some(other) = idx.dense_of(node_sparse_row(nb.node_id)) {
                uf.union(d, other);
            }
        }
        // `in_neighbors` is redundant after every out-edge is unioned (each
        // edge appears once in `out_neighbors(source)` and once in
        // `in_neighbors(target)`), but iterating in_neighbors as well does no
        // harm and guarantees the undirected closure even if a future
        // projection ever stored asymmetric out/in adjacency.
        for nb in proj.in_neighbors(nid) {
            if let Some(other) = idx.dense_of(node_sparse_row(nb.node_id)) {
                uf.union(d, other);
            }
        }
    }
}

/// Path-compressing union-find with union-by-rank. Sized by live-node count.
struct UnionFind {
    parent: Vec<u32>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        let parent = (0..size as u32).collect();
        Self {
            parent,
            rank: vec![0; size],
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let grand = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = grand;
            x = grand;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let (small, large) = if self.rank[ra as usize] < self.rank[rb as usize] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent[small as usize] = large;
        if self.rank[small as usize] == self.rank[large as usize] {
            self.rank[large as usize] += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Strongly Connected Components (Tarjan, iterative)
// ---------------------------------------------------------------------------

/// Compute strongly connected components — `(NodeId, component_id)` pairs where
/// `component_id` is the smallest `NodeId` in each SCC.
///
/// Uses **directed** adjacency (only `out_neighbors`). Returns an empty `Vec`
/// for an empty projection. Pairs are sorted ASC by `NodeId` per spec 16 §E12.
#[must_use]
pub fn scc(proj: &GraphProjection) -> Vec<(NodeId, u64)> {
    let idx = RowIndex::new(proj);
    let state = run_tarjan(proj, &idx);

    let mut result: Vec<(NodeId, u64)> = Vec::with_capacity(idx.len());
    for component in &state.components {
        let min_node = component
            .iter()
            .map(|&d| idx.node_id_of(d).get())
            .min()
            .expect("SCC component is non-empty");
        for &d in component {
            result.push((idx.node_id_of(d), min_node));
        }
    }
    result.sort_by_key(|&(nid, _)| nid.get());
    result
}

/// Count strongly connected components.
#[must_use]
pub fn scc_count(proj: &GraphProjection) -> usize {
    let idx = RowIndex::new(proj);
    let state = run_tarjan(proj, &idx);
    state.components.len()
}

struct TarjanState {
    /// Monotonic discovery counter.
    index: u32,
    /// Stack of in-progress dense indices (Tarjan's "S").
    stack: Vec<u32>,
    /// `on_stack[dense]` = true while `dense` is on `stack`.
    on_stack: Vec<bool>,
    /// `indices[dense]` = discovery time; `SENTINEL` while unvisited.
    indices: Vec<u32>,
    /// `lowlinks[dense]` = lowest discovery time reachable from `dense`'s
    /// subtree.
    lowlinks: Vec<u32>,
    /// Completed components, one `Vec<u32>` (dense indices) per SCC.
    components: Vec<Vec<u32>>,
}

impl TarjanState {
    fn with_capacity(size: usize) -> Self {
        Self {
            index: 0,
            stack: Vec::new(),
            on_stack: vec![false; size],
            indices: vec![SENTINEL; size],
            lowlinks: vec![SENTINEL; size],
            components: Vec::new(),
        }
    }
}

fn run_tarjan(proj: &GraphProjection, idx: &RowIndex) -> TarjanState {
    let mut state = TarjanState::with_capacity(idx.len());
    for d in 0..idx.len() as u32 {
        if state.indices[d as usize] == SENTINEL {
            tarjan_strongconnect(&mut state, d, proj, idx);
        }
    }
    state
}

/// Iterative Tarjan strongconnect with an explicit call stack (donor pattern,
/// `aether-db-algorithms/src/structural.rs:193-258`).
fn tarjan_strongconnect(
    state: &mut TarjanState,
    start: u32,
    proj: &GraphProjection,
    idx: &RowIndex,
) {
    // Frame: (current_dense, next_neighbor_index_into_cached_list).
    let mut call_stack: Vec<(u32, usize)> = Vec::new();
    // Per-DFS neighbor cache: dense → list of out-neighbor dense indices.
    // Filled lazily on first visit so we don't re-walk `proj.out_neighbors` on
    // every resume-from-child iteration. Neighbors outside the projection
    // scope are dropped at build time.
    let mut neighbors_cache: HashMap<u32, Vec<u32>> = HashMap::default();

    state.indices[start as usize] = state.index;
    state.lowlinks[start as usize] = state.index;
    state.index += 1;
    state.stack.push(start);
    state.on_stack[start as usize] = true;
    call_stack.push((start, 0));

    while let Some(&mut (v, ref mut ni)) = call_stack.last_mut() {
        let neighbors = neighbors_cache.entry(v).or_insert_with(|| {
            proj.out_neighbors(idx.node_id_of(v))
                .iter()
                .filter_map(|nb| idx.dense_of(node_sparse_row(nb.node_id)))
                .collect()
        });

        if *ni < neighbors.len() {
            let w = neighbors[*ni];
            *ni += 1;
            let wi = w as usize;

            if state.indices[wi] == SENTINEL {
                state.indices[wi] = state.index;
                state.lowlinks[wi] = state.index;
                state.index += 1;
                state.stack.push(w);
                state.on_stack[wi] = true;
                call_stack.push((w, 0));
            } else if state.on_stack[wi] {
                let vi = v as usize;
                state.lowlinks[vi] = state.lowlinks[vi].min(state.indices[wi]);
            }
        } else {
            // All neighbors of `v` processed; finish this frame.
            let finished = v;
            call_stack.pop();
            let fi = finished as usize;

            if let Some(&mut (parent, _)) = call_stack.last_mut() {
                let pi = parent as usize;
                state.lowlinks[pi] = state.lowlinks[pi].min(state.lowlinks[fi]);
            }

            if state.lowlinks[fi] == state.indices[fi] {
                // `finished` is the root of an SCC; pop the stack until we
                // hit it.
                let mut component = Vec::new();
                while let Some(w) = state.stack.pop() {
                    state.on_stack[w as usize] = false;
                    component.push(w);
                    if w == finished {
                        break;
                    }
                }
                state.components.push(component);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a `NodeId` (1-based per selene-graph) to its sparse row index
/// (0-based). Used at the projection boundary to look up neighbor rows before
/// converting to dense indices via [`RowIndex`].
///
/// Why: `NodeId::new(0)` is the TOMBSTONE sentinel; alive nodes start at row
/// 0 with `NodeId(1)`. Spec 16 §E11.
#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
