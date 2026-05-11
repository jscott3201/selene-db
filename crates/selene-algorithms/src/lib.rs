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
//! BRIEF-50 landed the projection foundation; BRIEF-51 adds the
//! [`ProjectionCatalog`] named cache with generation-based staleness
//! detection. Subsequent briefs add modules for structural (WCC, SCC, …),
//! pathfinding (Dijkstra, SSSP, APSP), centrality (PageRank, betweenness),
//! and community (label propagation, Louvain, triangle count) algorithms.
//! See spec 16 §3 for the brief sequence.
//!
//! # Dependency boundary
//!
//! Per spec 16 §E01, this crate depends on [`selene_core`] and
//! [`selene_graph`] only — never on `selene-gql`, `selene-pack`, or
//! `selene-persist`. A future `selene-algorithms-pack` (out-of-tree v1.x)
//! adapts these algorithms to procedure-pack tiers; the algorithms crate
//! itself stays independent of the GQL surface.

pub mod catalog;
pub mod error;
pub mod pathfinding;
pub mod projection;
pub mod structural;

pub use catalog::{ProjectionCatalog, ProjectionRef};
pub use error::AlgorithmsError;
pub use pathfinding::{PathResult, PathfindingError, apsp, dijkstra, sssp};
pub use projection::{GraphProjection, ProjNeighbor, ProjectionConfig};
pub use structural::{
    TopoSortError, articulation_points, bridges, scc, scc_count, topological_sort, wcc, wcc_count,
};
