//! Graph algorithms for selene-db.
//!
//! All algorithms operate on a [`GraphProjection`] — a frozen, filtered view
//! of the live graph with cached CSR adjacency. Algorithms are pure functions
//! of `&GraphProjection`; they do not retain references to the underlying
//! [`selene_graph::SeleneGraph`] and observe no mutations after the projection
//! is built.
//!
//! # Crate organization
//!
//! [`GraphProjection`] is the foundation. [`ProjectionCatalog`] adds a named
//! cache with generation-based staleness detection. Algorithm modules cover
//! structural (WCC, SCC, …), pathfinding (Dijkstra, SSSP, APSP), centrality
//! (PageRank, betweenness), and community (label propagation, Louvain,
//! triangle count) algorithms. See Spec 16 §3 for the package shape.
//!
//! # Dependency boundary
//!
//! Per Spec 16 §E01, this crate depends on [`selene_core`] and
//! [`selene_graph`] only — never on `selene-gql` or `selene-persist`. The
//! native `selene-gql` procedure registry binds these algorithms to the
//! `CALL algo.*` surface; the algorithms crate itself stays independent of
//! the GQL surface.

pub mod api;
pub mod catalog;
pub mod centrality;
pub mod community;
pub mod error;
pub mod parallel;
pub mod pathfinding;
pub mod projection;
#[cfg(any(test, feature = "test-harness"))]
pub mod snapshot_summary;
pub mod structural;

pub use api::{ApiError, GraphAlgorithms, ProjectionInfo};
pub use catalog::{ProjectionCatalog, ProjectionRef};
pub use centrality::{
    BetweennessConfig, PageRankConfig, PageRankOrientation, betweenness, betweenness_with_checker,
    pagerank, pagerank_with_checker,
};
pub use community::{
    TriangleCountConfig, label_propagation, label_propagation_with_checker, louvain,
    louvain_with_checker, triangle_count, triangle_count_with_checker,
};
pub use error::{AlgorithmAborted, AlgorithmsError};
pub use parallel::{MAX_PARALLELISM_THREADS, Parallelism, ParallelismCapError};
pub use pathfinding::{
    ApspConfig, PathResult, PathfindingError, apsp, apsp_with_checker, dijkstra,
    dijkstra_with_checker, sssp, sssp_with_checker,
};
pub use projection::{GraphProjection, ProjNeighbor, ProjectionConfig};
#[cfg(any(test, feature = "test-harness"))]
pub use snapshot_summary::{
    AlgoResult, AlgoSnapshot, AlgoSnapshotInput, GraphSummary, algo_summary,
};
pub use structural::{
    TopoSortError, articulation_points, articulation_points_with_checker, bridges,
    bridges_with_checker, scc, scc_count, scc_count_with_checker, scc_with_checker,
    topological_sort, topological_sort_with_checker, wcc, wcc_count, wcc_count_with_checker,
    wcc_with_checker,
};
