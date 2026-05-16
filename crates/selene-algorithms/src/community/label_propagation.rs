//! Raghavan-style label propagation for community detection.
//!
//! Each node initially carries its own NodeId as label. Each iteration visits
//! nodes in **ASC NodeId order** and updates each in turn — an
//! **asynchronous-deterministic** schedule (donor pattern). Each node adopts
//! the label most common among its undirected neighbors (out ∪ in); the
//! per-iteration update is immediately visible to later nodes within the same
//! iteration. Tie-break is smallest label ID (spec 16 §E30). Converges when
//! no labels change in an iteration, or `max_iter` reached.
//!
//! State arrays sized by live-node count via `RowIndex` (§E26). Unit weights
//! only — donor pattern; weighted variants deferred to v1.x per §E25 / §J Q5.

// Integer-keyed hot-path maps use FxHashMap to avoid SipHash overhead.
use rustc_hash::FxHashMap as HashMap;
use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// Compute community assignments via asynchronous-deterministic label
/// propagation.
///
/// Returns `(NodeId, community_id)` pairs sorted **ASC by NodeId** per spec 16
/// §E27. Empty projection → `vec![]`. `max_iter == 0` returns the initial
/// state (each node in its own community).
///
/// The community_id surfaces as the smallest **NodeId** that survived the
/// propagation (labels are NodeId values throughout). Isolated nodes retain
/// their initial label (donor / Raghavan: empty-neighbor → skip).
#[must_use]
pub fn label_propagation(proj: &GraphProjection, max_iter: usize) -> Vec<(NodeId, u64)> {
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Vec::new();
    }
    let n = idx.len();

    // labels[d] = current label of dense node d, encoded as the NodeId u64.
    // Initial label = each node's own NodeId so that converged labels are
    // directly interpretable as community IDs (smallest surviving NodeId per
    // component).
    let mut labels: Vec<u64> = (0..n as u32).map(|d| idx.node_id_of(d).get()).collect();

    // Reused per-node scratch; cleared at the top of each node iteration to
    // avoid reallocation in the hot loop.
    let mut label_counts: HashMap<u64, usize> = HashMap::default();

    for _ in 0..max_iter {
        let mut changed = false;

        for d in 0..n as u32 {
            let node = idx.node_id_of(d);
            label_counts.clear();

            // Multiplicity-faithful: count each directed half-edge separately
            // per §E25. Parallel edges therefore contribute multiple times.
            for nb in proj.out_neighbors(node) {
                if let Some(nd) = idx.dense_of(node_sparse_row(nb.node_id)) {
                    *label_counts.entry(labels[nd as usize]).or_insert(0) += 1;
                }
            }
            for nb in proj.in_neighbors(node) {
                if let Some(nd) = idx.dense_of(node_sparse_row(nb.node_id)) {
                    *label_counts.entry(labels[nd as usize]).or_insert(0) += 1;
                }
            }

            if label_counts.is_empty() {
                continue;
            }

            // Pick the most common label; on ties, smallest label ID wins
            // (§E30). `filter(count == max_count).map(label).min()` is the
            // donor formulation — explicit + deterministic.
            let max_count = *label_counts.values().max().expect("non-empty");
            let best_label = label_counts
                .iter()
                .filter(|&(_, &count)| count == max_count)
                .map(|(&label, _)| label)
                .min()
                .expect("non-empty after max");

            if labels[d as usize] != best_label {
                labels[d as usize] = best_label;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let mut result: Vec<(NodeId, u64)> = (0..n as u32)
        .map(|d| (idx.node_id_of(d), labels[d as usize]))
        .collect();
    result.sort_by_key(|&(nid, _)| nid.get());
    result
}

/// `NodeId` (1-based) → sparse row index (0-based) at the projection boundary.
/// Mirrors the helper in `structural::components`; centralizing it later is a
/// v1.x refactor (small enough function that local copies are cheap).
#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
