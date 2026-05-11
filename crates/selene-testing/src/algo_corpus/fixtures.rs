//! Closed enumeration of corpus fixture inputs for the algorithm snapshot
//! harness.
//!
//! Pure-mirror per spec 16 §E33: no `selene_algorithms` imports. Fixture
//! data is `&'static`-rooted so corpus entries can be `const`-friendly;
//! runtime instantiation (graph build + projection build + invocation
//! dispatch) happens in the test harness in `selene-algorithms`.

#![allow(missing_docs)]

/// Closed enumeration of graph shapes the harness instantiates against a
/// fresh `SharedGraph`. Each variant produces a deterministic node + edge
/// allocation order per spec 16 §E35.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum AlgoCorpusGraph {
    /// Empty graph (zero nodes). Tests every algorithm's empty-projection
    /// path per spec 16 §E09.
    Empty,
    /// Single isolated node. Tests the singleton contract for every
    /// algorithm.
    Single,
    /// Directed chain of N nodes: 0→1→2→…→N-1. The canonical pathfinding
    /// fixture.
    Chain { len: usize },
    /// Directed 3-cycle: 0→1, 1→2, 2→0. Used by triangle and centrality
    /// fixtures.
    K3DirectedCycle,
    /// Bidirectional K4 (complete graph on 4 nodes; 12 directed edges).
    K4Bidir,
    /// Bidirectional star: a center node connected to `spokes` leaves via
    /// `2 * spokes` directed edges.
    StarBidir { spokes: usize },
    /// Two bidirectional K3 triangles joined by one bidirectional bridge.
    /// 6 nodes, 14 directed edges. The canonical community-detection /
    /// WCC fixture.
    TwoTrianglesBridge,
    /// Sparse-row exercise: `total_nodes` nodes allocated, but the
    /// projection is scoped to `in_scope` rows via a `RoaringBitmap`.
    /// Tests spec 16 §E26 (RowIndex-based state sizing).
    SparseRowTriangle {
        total_nodes: usize,
        in_scope: &'static [u32],
    },
}

/// Closed enumeration of per-algorithm invocations. Each variant carries the
/// algorithm's required parameters. `source_row` values are **sparse row
/// indices** (0-based); the harness converts to `NodeId` via `row + 1`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum AlgoCorpusInvocation {
    // Structural
    Wcc,
    WccCount,
    Scc,
    SccCount,
    TopologicalSort,
    ArticulationPoints,
    Bridges,
    // Pathfinding
    Dijkstra {
        /// Sparse row index of the source node (0-based).
        from_row: u32,
        /// Sparse row index of the target node (0-based).
        to_row: u32,
    },
    Sssp {
        /// Sparse row index of the source node (0-based).
        source_row: u32,
    },
    Apsp {
        max_nodes: usize,
    },
    // Centrality
    Pagerank {
        damping: f64,
        max_iter: usize,
        tolerance: f64,
    },
    Betweenness {
        sample_size: Option<usize>,
    },
    // Community
    LabelPropagation {
        max_iter: usize,
    },
    Louvain {
        max_iter: usize,
    },
    TriangleCount,
}

impl AlgoCorpusInvocation {
    /// The algorithm surface this invocation exercises.
    /// Used by the dispatcher's `covered_algorithms` accounting.
    #[must_use]
    pub fn surface(self) -> super::AlgoSurface {
        use super::AlgoSurface;
        match self {
            AlgoCorpusInvocation::Wcc => AlgoSurface::Wcc,
            AlgoCorpusInvocation::WccCount => AlgoSurface::WccCount,
            AlgoCorpusInvocation::Scc => AlgoSurface::Scc,
            AlgoCorpusInvocation::SccCount => AlgoSurface::SccCount,
            AlgoCorpusInvocation::TopologicalSort => AlgoSurface::TopologicalSort,
            AlgoCorpusInvocation::ArticulationPoints => AlgoSurface::ArticulationPoints,
            AlgoCorpusInvocation::Bridges => AlgoSurface::Bridges,
            AlgoCorpusInvocation::Dijkstra { .. } => AlgoSurface::Dijkstra,
            AlgoCorpusInvocation::Sssp { .. } => AlgoSurface::Sssp,
            AlgoCorpusInvocation::Apsp { .. } => AlgoSurface::Apsp,
            AlgoCorpusInvocation::Pagerank { .. } => AlgoSurface::Pagerank,
            AlgoCorpusInvocation::Betweenness { .. } => AlgoSurface::Betweenness,
            AlgoCorpusInvocation::LabelPropagation { .. } => AlgoSurface::LabelPropagation,
            AlgoCorpusInvocation::Louvain { .. } => AlgoSurface::Louvain,
            AlgoCorpusInvocation::TriangleCount => AlgoSurface::TriangleCount,
        }
    }
}
