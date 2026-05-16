//! Curated graph-algorithm snapshot corpus fixtures (D21 pure-mirror DSL).
//!
//! Pure-mirror per spec 16 §E33: corpus types live in `selene-testing` and
//! have **zero imports** from `selene-algorithms` production types. The
//! drift test `algo_surface_mirror_matches_selene_algorithms_exports` (in
//! `crates/selene-algorithms/tests/algo_snapshot_corpus.rs`) keeps the
//! mirror in sync with the algorithm-export surface.

pub mod coverage;
pub mod fixtures;

pub use coverage::{ALGORITHM_COVERAGE, AlgoSurface};
pub use fixtures::{AlgoCorpusGraph, AlgoCorpusInvocation};

/// Curated corpus of algorithm snapshot entries.
#[derive(Clone, Debug)]
pub struct AlgoCorpus {
    entries: Vec<AlgoCorpusEntry>,
}

/// One entry in the algorithm snapshot corpus.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct AlgoCorpusEntry {
    /// Snake-case slug used as the snapshot file suffix.
    pub slug: &'static str,
    /// Short description; surfaces in test failure messages.
    pub description: &'static str,
    /// Graph fixture for this entry.
    pub graph: AlgoCorpusGraph,
    /// Algorithm invocation against the projection built from `graph`.
    pub invocation: AlgoCorpusInvocation,
    /// Category bucket — drift test ensures every variant is exercised.
    pub category: AlgoCorpusCategory,
    /// Algorithm surfaces this entry covers (typically `&[invocation.surface()]`;
    /// listed explicitly so the corpus coverage table is auditable without
    /// re-running the dispatcher).
    pub covered_algorithms: &'static [AlgoSurface],
}

/// Coarse-grained algorithm-family bucket. Drift test
/// `corpus_categories_covered` asserts every variant has ≥1 entry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AlgoCorpusCategory {
    /// Structural: WCC, SCC, topological sort, articulation points, bridges.
    Structural,
    /// Pathfinding: Dijkstra, SSSP, APSP.
    Pathfinding,
    /// Centrality: PageRank, betweenness.
    Centrality,
    /// Community: label propagation, Louvain, triangle count.
    Community,
}

impl AlgoCorpus {
    /// The curated algorithm closure corpus. ≥18 entries covering every algorithm
    /// surface plus per-family edge cases (empty / sparse-row / max_iter=0).
    #[must_use]
    pub fn m5f() -> Self {
        Self {
            entries: ALGO_ENTRIES.to_vec(),
        }
    }

    /// Iterate the entries in declaration order.
    pub fn entries(&self) -> impl Iterator<Item = &AlgoCorpusEntry> + '_ {
        self.entries.iter()
    }

    /// Number of entries in this corpus.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the corpus is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Algorithm entries
// ---------------------------------------------------------------------------

const ALGO_ENTRIES: &[AlgoCorpusEntry] = &[
    // === Structural (7 algorithm surfaces + 1 sparse-row + 1 empty) ===
    AlgoCorpusEntry {
        slug: "structural_wcc_two_triangles_bridge",
        description: "WCC on two-triangle-with-bridge: single WCC under undirected closure.",
        graph: AlgoCorpusGraph::TwoTrianglesBridge,
        invocation: AlgoCorpusInvocation::Wcc,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::Wcc],
    },
    AlgoCorpusEntry {
        slug: "structural_wcc_count_chain_5",
        description: "WCC count on chain n=5 → 1 component.",
        graph: AlgoCorpusGraph::Chain { len: 5 },
        invocation: AlgoCorpusInvocation::WccCount,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::WccCount],
    },
    AlgoCorpusEntry {
        slug: "structural_scc_k3_cycle",
        description: "SCC on directed 3-cycle: 1 strongly-connected component.",
        graph: AlgoCorpusGraph::K3DirectedCycle,
        invocation: AlgoCorpusInvocation::Scc,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::Scc],
    },
    AlgoCorpusEntry {
        slug: "structural_scc_count_chain_4",
        description: "SCC count on chain n=4 → 4 trivial SCCs (no cycle).",
        graph: AlgoCorpusGraph::Chain { len: 4 },
        invocation: AlgoCorpusInvocation::SccCount,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::SccCount],
    },
    AlgoCorpusEntry {
        slug: "structural_topological_sort_chain_5",
        description: "Topological sort on directed chain n=5 → deterministic order.",
        graph: AlgoCorpusGraph::Chain { len: 5 },
        invocation: AlgoCorpusInvocation::TopologicalSort,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::TopologicalSort],
    },
    AlgoCorpusEntry {
        slug: "structural_articulation_points_two_triangles_bridge",
        description: "Articulation points on two-triangle-bridge: 2 cut vertices (bridge endpoints).",
        graph: AlgoCorpusGraph::TwoTrianglesBridge,
        invocation: AlgoCorpusInvocation::ArticulationPoints,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::ArticulationPoints],
    },
    AlgoCorpusEntry {
        slug: "structural_bridges_two_triangles_bridge",
        description: "Bridges on two-triangle-bridge: 1 bridge (the joining edge).",
        graph: AlgoCorpusGraph::TwoTrianglesBridge,
        invocation: AlgoCorpusInvocation::Bridges,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[AlgoSurface::Bridges],
    },
    // === Pathfinding (3 surfaces) ===
    AlgoCorpusEntry {
        slug: "pathfinding_dijkstra_chain_5_from_0_to_4",
        description: "Dijkstra on directed chain n=5 from row 0 to row 4: path [0,1,2,3,4] cost 4.",
        graph: AlgoCorpusGraph::Chain { len: 5 },
        invocation: AlgoCorpusInvocation::Dijkstra {
            from_row: 0,
            to_row: 4,
        },
        category: AlgoCorpusCategory::Pathfinding,
        covered_algorithms: &[AlgoSurface::Dijkstra],
    },
    AlgoCorpusEntry {
        slug: "pathfinding_sssp_chain_5_from_0",
        description: "SSSP on directed chain n=5 from row 0: hop counts 0..4.",
        graph: AlgoCorpusGraph::Chain { len: 5 },
        invocation: AlgoCorpusInvocation::Sssp { source_row: 0 },
        category: AlgoCorpusCategory::Pathfinding,
        covered_algorithms: &[AlgoSurface::Sssp],
    },
    AlgoCorpusEntry {
        slug: "pathfinding_apsp_k3_cycle_max_10",
        description: "APSP on directed 3-cycle: 6 reachable pairs (excluding self).",
        graph: AlgoCorpusGraph::K3DirectedCycle,
        invocation: AlgoCorpusInvocation::Apsp { max_nodes: 10 },
        category: AlgoCorpusCategory::Pathfinding,
        covered_algorithms: &[AlgoSurface::Apsp],
    },
    // === Centrality (2 surfaces + 1 edge-case zero-damping) ===
    AlgoCorpusEntry {
        slug: "centrality_pagerank_star_bidir_5_default",
        description: "PageRank on bidirectional 5-spoke star: center dominates.",
        graph: AlgoCorpusGraph::StarBidir { spokes: 5 },
        invocation: AlgoCorpusInvocation::Pagerank {
            damping: 0.85,
            max_iter: 100,
            tolerance: 1e-6,
        },
        category: AlgoCorpusCategory::Centrality,
        covered_algorithms: &[AlgoSurface::Pagerank],
    },
    AlgoCorpusEntry {
        slug: "centrality_pagerank_chain_3_zero_damping",
        description: "PageRank zero-damping edge case: pure teleport → uniform 1/N.",
        graph: AlgoCorpusGraph::Chain { len: 3 },
        invocation: AlgoCorpusInvocation::Pagerank {
            damping: 0.0,
            max_iter: 10,
            tolerance: 1e-6,
        },
        category: AlgoCorpusCategory::Centrality,
        covered_algorithms: &[AlgoSurface::Pagerank],
    },
    AlgoCorpusEntry {
        slug: "centrality_betweenness_k3_cycle_full",
        description: "Betweenness on directed 3-cycle: all 3 nodes tied at 1.0 by symmetry.",
        graph: AlgoCorpusGraph::K3DirectedCycle,
        invocation: AlgoCorpusInvocation::Betweenness { sample_size: None },
        category: AlgoCorpusCategory::Centrality,
        covered_algorithms: &[AlgoSurface::Betweenness],
    },
    // === Community (3 surfaces) ===
    AlgoCorpusEntry {
        slug: "community_label_propagation_two_triangles_bridge",
        description: "Label propagation on two-triangle-bridge: dense subgroups self-label.",
        graph: AlgoCorpusGraph::TwoTrianglesBridge,
        invocation: AlgoCorpusInvocation::LabelPropagation { max_iter: 50 },
        category: AlgoCorpusCategory::Community,
        covered_algorithms: &[AlgoSurface::LabelPropagation],
    },
    AlgoCorpusEntry {
        slug: "community_louvain_two_triangles_bridge",
        description: "Louvain on two-triangle-bridge: communities form per triangle.",
        graph: AlgoCorpusGraph::TwoTrianglesBridge,
        invocation: AlgoCorpusInvocation::Louvain { max_iter: 50 },
        category: AlgoCorpusCategory::Community,
        covered_algorithms: &[AlgoSurface::Louvain],
    },
    AlgoCorpusEntry {
        slug: "community_triangle_count_k4_bidir",
        description: "Triangle count on K4 bidir: every vertex in 3 triangles.",
        graph: AlgoCorpusGraph::K4Bidir,
        invocation: AlgoCorpusInvocation::TriangleCount,
        category: AlgoCorpusCategory::Community,
        covered_algorithms: &[AlgoSurface::TriangleCount],
    },
    // === Per-family edge cases ===
    AlgoCorpusEntry {
        slug: "edge_case_wcc_empty_projection",
        description: "Empty-projection contract (§E09): WCC on empty graph → empty result.",
        graph: AlgoCorpusGraph::Empty,
        invocation: AlgoCorpusInvocation::Wcc,
        category: AlgoCorpusCategory::Structural,
        covered_algorithms: &[],
    },
    AlgoCorpusEntry {
        slug: "edge_case_label_propagation_max_iter_zero",
        description: "Caller-supplied max_iter contract (§E28): max_iter=0 returns initial labels.",
        graph: AlgoCorpusGraph::K3DirectedCycle,
        invocation: AlgoCorpusInvocation::LabelPropagation { max_iter: 0 },
        category: AlgoCorpusCategory::Community,
        covered_algorithms: &[],
    },
    AlgoCorpusEntry {
        slug: "edge_case_triangle_count_sparse_row",
        description: "RowIndex sizing contract (§E26): triangle on rows {10,50,90} of 100-node graph.",
        graph: AlgoCorpusGraph::SparseRowTriangle {
            total_nodes: 100,
            in_scope: &[10, 50, 90],
        },
        invocation: AlgoCorpusInvocation::TriangleCount,
        category: AlgoCorpusCategory::Community,
        covered_algorithms: &[],
    },
];
