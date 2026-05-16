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
//! [`selene_graph`] only — never on `selene-gql`, `selene-pack`, or
//! `selene-persist`. A future `selene-algorithms-pack` (out-of-tree v1.x)
//! adapts these algorithms to procedure-pack tiers; the algorithms crate
//! itself stays independent of the GQL surface.

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

pub use catalog::{ProjectionCatalog, ProjectionRef};
pub use centrality::{BetweennessConfig, PageRankConfig, betweenness, pagerank};
pub use community::{TriangleCountConfig, label_propagation, louvain, triangle_count};
pub use error::AlgorithmsError;
pub use parallel::Parallelism;
pub use pathfinding::{ApspConfig, PathResult, PathfindingError, apsp, dijkstra, sssp};
pub use projection::{GraphProjection, ProjNeighbor, ProjectionConfig};
#[cfg(any(test, feature = "test-harness"))]
pub use snapshot_summary::{
    AlgoResult, AlgoSnapshot, AlgoSnapshotInput, GraphSummary, algo_summary,
};
pub use structural::{
    TopoSortError, articulation_points, bridges, scc, scc_count, topological_sort, wcc, wcc_count,
};
