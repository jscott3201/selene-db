//! Drift-protection coverage tables for the algorithm snapshot corpus.
//!
//! Pure-mirror per spec 16 §E33: NO `selene_algorithms` type imports — this
//! enum is a stable corpus-local mirror of the 15 algorithm public surfaces
//! re-exported from `selene_algorithms::lib`. A drift test (in
//! `crates/selene-algorithms/tests/algo_snapshot_corpus.rs`) asserts the
//! mirror and the real export set stay in sync.

#![allow(missing_docs)]

/// Every algorithm public surface the corpus must exercise at least once.
pub const ALGORITHM_COVERAGE: &[AlgoSurface] = AlgoSurface::ALL;

/// Stable mirror of `selene_algorithms`'s public algorithm surfaces.
///
/// Excludes foundation types (`GraphProjection`, `ProjectionCatalog`) which
/// are not algorithms in the M5f spec sense — they are exercised by every
/// corpus entry's projection build but do not appear as `AlgoSurface`
/// variants.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AlgoSurface {
    // Structural (spec 16 §E09–§E12)
    Wcc,
    WccCount,
    Scc,
    SccCount,
    TopologicalSort,
    ArticulationPoints,
    Bridges,
    // Pathfinding (spec 16 §E13–§E18)
    Dijkstra,
    Sssp,
    Apsp,
    // Centrality (spec 16 §E19–§E24)
    Pagerank,
    Betweenness,
    // Community (spec 16 §E25–§E30)
    LabelPropagation,
    Louvain,
    TriangleCount,
}

impl AlgoSurface {
    /// All 15 algorithm surfaces in stable declaration order.
    pub const ALL: &'static [AlgoSurface] = &[
        AlgoSurface::Wcc,
        AlgoSurface::WccCount,
        AlgoSurface::Scc,
        AlgoSurface::SccCount,
        AlgoSurface::TopologicalSort,
        AlgoSurface::ArticulationPoints,
        AlgoSurface::Bridges,
        AlgoSurface::Dijkstra,
        AlgoSurface::Sssp,
        AlgoSurface::Apsp,
        AlgoSurface::Pagerank,
        AlgoSurface::Betweenness,
        AlgoSurface::LabelPropagation,
        AlgoSurface::Louvain,
        AlgoSurface::TriangleCount,
    ];

    /// Human-readable name; used by the snapshot renderer for the
    /// `INVOCATION { algorithm: "<name>" }` header line.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            AlgoSurface::Wcc => "wcc",
            AlgoSurface::WccCount => "wcc_count",
            AlgoSurface::Scc => "scc",
            AlgoSurface::SccCount => "scc_count",
            AlgoSurface::TopologicalSort => "topological_sort",
            AlgoSurface::ArticulationPoints => "articulation_points",
            AlgoSurface::Bridges => "bridges",
            AlgoSurface::Dijkstra => "dijkstra",
            AlgoSurface::Sssp => "sssp",
            AlgoSurface::Apsp => "apsp",
            AlgoSurface::Pagerank => "pagerank",
            AlgoSurface::Betweenness => "betweenness",
            AlgoSurface::LabelPropagation => "label_propagation",
            AlgoSurface::Louvain => "louvain",
            AlgoSurface::TriangleCount => "triangle_count",
        }
    }
}
