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

use rayon::prelude::*;
use selene_core::{CancellationChecker, NodeId};

use crate::error::{AlgorithmAborted, check_algorithm, check_algorithm_stride};
use crate::parallel::{ParallelRunner, Parallelism};
use crate::projection::GraphProjection;
use crate::structural::{RowIndex, SENTINEL};

/// Configuration for betweenness centrality.
#[derive(Debug, Clone, Copy, Default)]
pub struct BetweennessConfig {
    /// Optional deterministic sample size for approximate betweenness.
    pub sample_size: Option<usize>,
    /// Requested parallel execution policy.
    pub parallelism: Parallelism,
}

/// Compute betweenness centrality for every node in the projection.
///
/// Returns `(NodeId, score)` pairs sorted **DESC by score** with **NodeId
/// ASC** tie-break on equal scores (spec 16 §E21). Empty projection →
/// `vec![]`.
///
/// When `sample_size` is `Some(k)` with `0 < k < node_count`, sources are
/// sampled deterministically using endpoint-aware spacing and the final
/// centrality is scaled by `node_count / k` per §E24. `Some(k)` with `k >=
/// node_count` is equivalent to `None`. `Some(0)` returns zero centrality for
/// every node.
#[must_use]
pub fn betweenness(proj: &GraphProjection, config: BetweennessConfig) -> Vec<(NodeId, f64)> {
    betweenness_with_checker(proj, config, CancellationChecker::disabled())
        .expect("disabled cancellation checker never aborts")
}

/// Compute betweenness centrality with cooperative cancellation checkpoints.
pub fn betweenness_with_checker(
    proj: &GraphProjection,
    config: BetweennessConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, AlgorithmAborted> {
    match config.parallelism {
        Parallelism::Sequential => betweenness_sequential(proj, config.sample_size, checker),
        Parallelism::Auto | Parallelism::Threads(_) => {
            betweenness_parallel(proj, config.sample_size, config.parallelism, checker)
        }
    }
}

fn betweenness_sequential(
    proj: &GraphProjection,
    sample_size: Option<usize>,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, AlgorithmAborted> {
    check_algorithm(checker)?;
    let adjacency = DenseAdjacency::new(proj);
    let n = adjacency.len();
    if n == 0 {
        return Ok(Vec::new());
    }

    let (sources, scale) = compute_sample_sources(n, sample_size);
    let mut state = WorkerState::new(n);
    let mut sources_since_check = 0usize;
    for source in sources {
        check_algorithm_stride(checker, &mut sources_since_check)?;
        accumulate_brandes_at_source(&adjacency, source, &mut state, checker)?;
    }
    apply_sample_scaling(&mut state.centrality, scale);
    Ok(project_and_sort_centrality_pairs(
        &adjacency,
        state.centrality,
    ))
}

fn betweenness_parallel(
    proj: &GraphProjection,
    sample_size: Option<usize>,
    parallelism: Parallelism,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, AlgorithmAborted> {
    check_algorithm(checker)?;
    let adjacency = DenseAdjacency::new(proj);
    let n = adjacency.len();
    if n == 0 {
        return Ok(Vec::new());
    }

    let (sources, scale) = compute_sample_sources(n, sample_size);
    let runner =
        ParallelRunner::new(parallelism).expect("ParallelRunner builds for valid parallelism");
    let mut state = runner.install(|| {
        sources
            .into_par_iter()
            .fold(
                || Ok::<WorkerState, AlgorithmAborted>(WorkerState::new(n)),
                |state: Result<WorkerState, AlgorithmAborted>, source| {
                    let mut state = state?;
                    check_algorithm(checker)?;
                    accumulate_brandes_at_source(&adjacency, source, &mut state, checker)?;
                    Ok(state)
                },
            )
            .reduce(
                || Ok::<WorkerState, AlgorithmAborted>(WorkerState::new(n)),
                |left, right| match (left, right) {
                    (Ok(left), Ok(right)) => Ok(WorkerState::merge(left, right)),
                    (Err(error), _) | (_, Err(error)) => Err(error),
                },
            )
    })?;

    apply_sample_scaling(&mut state.centrality, scale);
    Ok(project_and_sort_centrality_pairs(
        &adjacency,
        state.centrality,
    ))
}

struct DenseAdjacency {
    idx: RowIndex,
    out_neighbors_dense: Vec<Vec<u32>>,
}

impl DenseAdjacency {
    fn new(proj: &GraphProjection) -> Self {
        let idx = RowIndex::new(proj);
        let n = idx.len();
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
        Self {
            idx,
            out_neighbors_dense,
        }
    }

    fn len(&self) -> usize {
        self.out_neighbors_dense.len()
    }

    fn node_id_of(&self, dense: u32) -> NodeId {
        self.idx.node_id_of(dense)
    }

    fn out_neighbors(&self, dense: u32) -> &[u32] {
        &self.out_neighbors_dense[dense as usize]
    }
}

struct WorkerState {
    centrality: Vec<f64>,
    pred: Vec<Vec<u32>>,
    sigma: Vec<f64>,
    dist: Vec<u32>,
    delta: Vec<f64>,
    queue: VecDeque<u32>,
    stack: Vec<u32>,
}

impl WorkerState {
    fn new(n: usize) -> Self {
        Self {
            centrality: vec![0.0; n],
            pred: (0..n).map(|_| Vec::new()).collect(),
            sigma: vec![0.0; n],
            dist: vec![SENTINEL; n],
            delta: vec![0.0; n],
            queue: VecDeque::with_capacity(n),
            stack: Vec::with_capacity(n),
        }
    }

    fn reset_for_source(&mut self) {
        for v in 0..self.centrality.len() {
            self.pred[v].clear();
            self.sigma[v] = 0.0;
            self.dist[v] = SENTINEL;
            self.delta[v] = 0.0;
        }
        self.queue.clear();
        self.stack.clear();
    }

    fn merge(mut a: Self, b: Self) -> Self {
        debug_assert_eq!(a.centrality.len(), b.centrality.len());
        for (x, y) in a.centrality.iter_mut().zip(&b.centrality) {
            *x += y;
        }
        a
    }
}

fn compute_sample_sources(n: usize, sample_size: Option<usize>) -> (Vec<u32>, f64) {
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
    match sample_size {
        Some(0) => (Vec::new(), 1.0),
        Some(1) if n > 1 => (vec![0u32], n as f64),
        Some(k) if k < n && k >= 2 => {
            let span = n - 1;
            let divisor = k - 1;
            let sampled: Vec<u32> = (0..k).map(|i| ((i * span) / divisor) as u32).collect();
            (sampled, n as f64 / k as f64)
        }
        _ => ((0..n as u32).collect(), 1.0),
    }
}

fn accumulate_brandes_at_source(
    adjacency: &DenseAdjacency,
    source: u32,
    state: &mut WorkerState,
    checker: CancellationChecker<'_>,
) -> Result<(), AlgorithmAborted> {
    state.reset_for_source();
    let si = source as usize;
    state.sigma[si] = 1.0;
    state.dist[si] = 0;
    state.queue.push_back(source);

    // BFS phase: discover shortest paths.
    let mut rows_since_check = 0usize;
    while let Some(v) = state.queue.pop_front() {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        state.stack.push(v);
        let vi = v as usize;
        let d_v = state.dist[vi];
        for &w in adjacency.out_neighbors(v) {
            let wi = w as usize;
            if state.dist[wi] == SENTINEL {
                state.queue.push_back(w);
                state.dist[wi] = d_v + 1;
            }
            if state.dist[wi] == d_v + 1 {
                state.sigma[wi] += state.sigma[vi];
                state.pred[wi].push(v);
            }
        }
    }

    // Dependency accumulation: walk back through the stack in reverse BFS order.
    while let Some(w) = state.stack.pop() {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let wi = w as usize;
        for &v in &state.pred[wi] {
            let vi = v as usize;
            // Why: sigma[v] / sigma[w] is well-defined because sigma[w] is
            // positive when pred[w] is non-empty.
            let increment = (state.sigma[vi] / state.sigma[wi]) * (1.0 + state.delta[wi]);
            state.delta[vi] += increment;
        }
        if w != source {
            state.centrality[wi] += state.delta[wi];
        }
    }
    Ok(())
}

fn apply_sample_scaling(centrality: &mut [f64], scale: f64) {
    if scale != 1.0 {
        for slot in centrality {
            *slot *= scale;
        }
    }
}

fn project_and_sort_centrality_pairs(
    adjacency: &DenseAdjacency,
    centrality: Vec<f64>,
) -> Vec<(NodeId, f64)> {
    let mut result: Vec<(NodeId, f64)> = centrality
        .into_iter()
        .enumerate()
        .map(|(d, score)| (adjacency.node_id_of(d as u32), score))
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
