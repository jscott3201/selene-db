//! Brandes' betweenness centrality.
//!
//! For each source s, BFS the directed graph to compute σ (shortest-path
//! counts) and δ (dependencies). Accumulate δ values across all sources into
//! `centrality[]`. Optional `sample_size` for approximate computation with
//! linear scaling per spec 16 §E24.
//!
//! State arrays sized by live count via `RowIndex` (§E20). Result sorted
//! DESC by score with NodeId ASC tie-break (§E21).
//!
//! ## Predecessor list determinism (§O.W.1)
//!
//! `pred[w]` accumulates ALL shortest-path predecessors of `w` from the
//! current source. Insertion order matches BFS visit order; since BFS pushes
//! neighbors in `out_neighbors` order (ASC by NodeId per §E03) and the queue
//! is FIFO, predecessor insertion is deterministic. The dependency δ
//! accumulation in the reverse-stack phase is order-invariant in the math
//! (sigma[v] / sigma[w] * (1 + delta[w]) is associative for the sum), so no
//! explicit predecessor tie-break is needed.

use std::collections::VecDeque;

use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::{RowIndex, SENTINEL};

/// Compute betweenness centrality for every node in the projection.
///
/// Returns `(NodeId, score)` pairs sorted **DESC by score** with **NodeId
/// ASC** tie-break on equal scores (spec 16 §E21). Empty projection →
/// `vec![]`.
///
/// When `sample_size` is `Some(k)` with `0 < k < node_count`, sources are
/// sampled deterministically (`step = node_count / k`) and the final
/// centrality is scaled by `node_count / k` per §E24. `Some(k)` with
/// `k >= node_count` is equivalent to `None`. `Some(0)` returns zero
/// centrality for every node.
#[must_use]
pub fn betweenness(proj: &GraphProjection, sample_size: Option<usize>) -> Vec<(NodeId, f64)> {
    let idx = RowIndex::new(proj);
    let n = idx.len();
    if n == 0 {
        return Vec::new();
    }

    // Cache directed out-neighbor lists once (BFS revisits these per source).
    let mut out_neighbors_dense: Vec<Vec<u32>> = Vec::with_capacity(n);
    for d in 0..n as u32 {
        let node = idx.node_id_of(d);
        let neighbors: Vec<u32> = proj
            .out_neighbors(node)
            .iter()
            .filter_map(|nb| idx.dense_of(node_sparse_row(nb.node_id)))
            .collect();
        out_neighbors_dense.push(neighbors);
    }

    let mut centrality: Vec<f64> = vec![0.0; n];

    // Determine source dense indices per §E24.
    //
    // Why endpoint-aware spacing (PR #60 Codex P2):
    // The donor's `step = n / k` indexing biases toward low dense indices
    // because integer floor division never reaches the tail. For example
    // `n=5, k=4` would yield sources `[0, 1, 2, 3]` and never sample dense
    // index 4 — systematically skewing approximate betweenness by NodeId
    // ordering rather than graph structure. Use `i * (n - 1) / (k - 1)`
    // (integer math) instead, which lands at both endpoints and spreads the
    // intermediate samples evenly: `n=5, k=4` → `[0, 1, 2, 4]`; `n=10, k=3`
    // → `[0, 4, 9]`. The k == 1 case degenerates to a single sample at
    // index 0 (the formula divides by zero otherwise).
    let (sources, scale): (Vec<u32>, f64) = match sample_size {
        Some(0) => (Vec::new(), 1.0),
        Some(1) if n > 1 => (vec![0u32], n as f64),
        Some(k) if k < n && k >= 2 => {
            let span = n - 1;
            let divisor = k - 1;
            let sampled: Vec<u32> = (0..k).map(|i| ((i * span) / divisor) as u32).collect();
            (sampled, n as f64 / k as f64)
        }
        _ => ((0..n as u32).collect(), 1.0),
    };

    // Per-iteration state arrays — allocated once, cleared between sources.
    let mut pred: Vec<Vec<u32>> = (0..n).map(|_| Vec::new()).collect();
    let mut sigma: Vec<f64> = vec![0.0; n];
    let mut dist: Vec<u32> = vec![SENTINEL; n];
    let mut delta: Vec<f64> = vec![0.0; n];

    for &s in &sources {
        // Reset per-source state. Why: clearing in-place is O(n) per source
        // and avoids reallocation; per-source vectors are wasteful.
        for v in 0..n {
            pred[v].clear();
            sigma[v] = 0.0;
            dist[v] = SENTINEL;
            delta[v] = 0.0;
        }

        let si = s as usize;
        sigma[si] = 1.0;
        dist[si] = 0;
        let mut queue: VecDeque<u32> = VecDeque::new();
        let mut stack: Vec<u32> = Vec::new();
        queue.push_back(s);

        // BFS phase: discover shortest paths.
        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let vi = v as usize;
            let d_v = dist[vi];
            for &w in &out_neighbors_dense[vi] {
                let wi = w as usize;
                if dist[wi] == SENTINEL {
                    queue.push_back(w);
                    dist[wi] = d_v + 1;
                }
                if dist[wi] == d_v + 1 {
                    sigma[wi] += sigma[vi];
                    pred[wi].push(v);
                }
            }
        }

        // Dependency accumulation: walk back through the stack in reverse
        // BFS order.
        while let Some(w) = stack.pop() {
            let wi = w as usize;
            for &v in &pred[wi] {
                let vi = v as usize;
                // Why: sigma[v] / sigma[w] is well-defined because sigma[w]
                // is positive (w was reached via at least one shortest path)
                // when pred[w] is non-empty.
                let increment = (sigma[vi] / sigma[wi]) * (1.0 + delta[wi]);
                delta[vi] += increment;
            }
            if w != s {
                centrality[wi] += delta[wi];
            }
        }
    }

    // Apply sample-size scaling (§E24). When scale == 1.0 this is a no-op
    // but the multiply is cheaper than branching.
    if scale != 1.0 {
        for slot in centrality.iter_mut() {
            *slot *= scale;
        }
    }

    // Materialize + sort DESC by score with NodeId ASC tie-break (§E21).
    let mut result: Vec<(NodeId, f64)> = (0..n as u32)
        .map(|d| (idx.node_id_of(d), centrality[d as usize]))
        .collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.get().cmp(&b.0.get())));
    result
}

/// Map a `NodeId` (1-based per selene-graph) to its sparse row index
/// (0-based).
#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
