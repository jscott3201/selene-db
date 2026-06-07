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

use selene_core::{CancellationChecker, NodeId};

use crate::error::{AlgorithmAborted, check_algorithm, check_algorithm_stride};
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
    wcc_with_checker(proj, CancellationChecker::disabled())
        .expect("disabled cancellation checker never aborts")
}

/// Compute weakly connected components with cooperative cancellation checkpoints.
pub fn wcc_with_checker(
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, u64)>, AlgorithmAborted> {
    check_algorithm(checker)?;
    let idx = proj.row_index();
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let mut uf = UnionFind::new(idx.len());
    union_all_edges_with_checker(proj, idx, &mut uf, checker)?;

    // Compute the minimum NodeId per current root to canonicalize component IDs
    // (donor `aether-db-algorithms/src/structural.rs:107-115`).
    let mut min_per_root = vec![u64::MAX; idx.len()];
    for d in 0..idx.len() as u32 {
        let root = uf.find(d) as usize;
        let node_id = idx.node_id_of(d).get();
        if node_id < min_per_root[root] {
            min_per_root[root] = node_id;
        }
    }

    // Second pass: rewrite root → min NodeId.
    let result: Vec<(NodeId, u64)> = (0..idx.len() as u32)
        .map(|d| {
            let root = uf.find(d) as usize;
            debug_assert_ne!(min_per_root[root], u64::MAX);
            (idx.node_id_of(d), min_per_root[root])
        })
        .collect();
    Ok(result)
}

/// Count weakly connected components without materializing per-node IDs.
///
/// Donor `aether-db-algorithms/src/structural.rs:121-136`: counts unique
/// union-find roots, avoiding the result-Vec allocation.
#[must_use]
pub fn wcc_count(proj: &GraphProjection) -> usize {
    wcc_count_with_checker(proj, CancellationChecker::disabled())
        .expect("disabled cancellation checker never aborts")
}

/// Count weakly connected components with cooperative cancellation checkpoints.
pub fn wcc_count_with_checker(
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<usize, AlgorithmAborted> {
    check_algorithm(checker)?;
    let idx = proj.row_index();
    if idx.is_empty() {
        return Ok(0);
    }
    let mut uf = UnionFind::new(idx.len());
    union_all_edges_with_checker(proj, idx, &mut uf, checker)?;

    let mut count = 0usize;
    let mut rows_since_check = 0usize;
    for d in 0..idx.len() as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        if uf.find(d) == d {
            count += 1;
        }
    }
    Ok(count)
}

/// Union every projection edge (directed → undirected) into `uf`. Edges whose
/// endpoints land outside the projection scope are skipped — matches the
/// in-degree counting guard in topological-sort.
fn union_all_edges_with_checker(
    proj: &GraphProjection,
    idx: &RowIndex,
    uf: &mut UnionFind,
    checker: CancellationChecker<'_>,
) -> Result<(), AlgorithmAborted> {
    let mut rows_since_check = 0usize;
    for d in 0..idx.len() as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        for nb in proj.out_neighbors_dense(d) {
            uf.union(d, nb.dense);
        }
        // The in-neighbor pass is redundant in release: every edge appears once
        // in `out_neighbors(source)` and once in `in_neighbors(target)`, and the
        // CSR transpose invariant (asserted in `GraphProjection::build`) keeps
        // out/in adjacency symmetric — so unioning out-edges already forms the
        // full undirected closure. We retain the in-neighbor union ONLY in debug
        // builds: it is a cheap cross-check of that invariant (if a future
        // projection ever stored asymmetric out/in adjacency, the debug build
        // still produces the correct undirected closure), while release skips
        // the duplicate union-find work.
        #[cfg(debug_assertions)]
        let nid = idx.node_id_of(d);
        #[cfg(debug_assertions)]
        for nb in proj.in_neighbors(nid) {
            uf.union(d, nb.dense);
        }
    }
    Ok(())
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
    scc_with_checker(proj, CancellationChecker::disabled())
        .expect("disabled cancellation checker never aborts")
}

/// Compute strongly connected components with cooperative cancellation checkpoints.
pub fn scc_with_checker(
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, u64)>, AlgorithmAborted> {
    check_algorithm(checker)?;
    let idx = proj.row_index();
    let state = run_tarjan(proj, idx, checker)?;

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
    Ok(result)
}

/// Count strongly connected components.
#[must_use]
pub fn scc_count(proj: &GraphProjection) -> usize {
    scc_count_with_checker(proj, CancellationChecker::disabled())
        .expect("disabled cancellation checker never aborts")
}

/// Count strongly connected components with cooperative cancellation checkpoints.
pub fn scc_count_with_checker(
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<usize, AlgorithmAborted> {
    check_algorithm(checker)?;
    let idx = proj.row_index();
    let state = run_tarjan(proj, idx, checker)?;
    Ok(state.components.len())
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

fn run_tarjan(
    proj: &GraphProjection,
    idx: &RowIndex,
    checker: CancellationChecker<'_>,
) -> Result<TarjanState, AlgorithmAborted> {
    let mut state = TarjanState::with_capacity(idx.len());
    let mut rows_since_check = 0usize;
    for d in 0..idx.len() as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        if state.indices[d as usize] == SENTINEL {
            tarjan_strongconnect(&mut state, d, proj, checker)?;
        }
    }
    Ok(state)
}

/// Iterative Tarjan strongconnect with an explicit call stack (donor pattern,
/// `aether-db-algorithms/src/structural.rs:193-258`).
fn tarjan_strongconnect(
    state: &mut TarjanState,
    start: u32,
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<(), AlgorithmAborted> {
    // Frame: (current_dense, next_neighbor_index_into_cached_list).
    let mut call_stack: Vec<(u32, usize)> = Vec::new();

    state.indices[start as usize] = state.index;
    state.lowlinks[start as usize] = state.index;
    state.index += 1;
    state.stack.push(start);
    state.on_stack[start as usize] = true;
    call_stack.push((start, 0));

    let mut rows_since_check = 0usize;
    while let Some(&mut (v, ref mut ni)) = call_stack.last_mut() {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let neighbors = proj.out_neighbors_dense(v);

        if *ni < neighbors.len() {
            let w = neighbors[*ni].dense;
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
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
