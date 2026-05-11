//! Structural graph algorithms: connected components, topological sort,
//! articulation points, and bridges.
//!
//! All algorithms operate on a [`crate::GraphProjection`] and return
//! deterministic outputs per spec 16 §E12. DFS-based algorithms (SCC,
//! articulation points, bridges) use iterative work-stack-based traversal
//! per spec 16 §E11 — no recursion, suitable for graphs of 10⁷ nodes.
//!
//! ## Algorithm directionality (spec 16 §E10)
//!
//! - [`wcc`], [`articulation_points`], [`bridges`] treat the projection as
//!   **undirected** (union of out-neighbors and in-neighbors per node, deduped
//!   then sorted by row).
//! - [`scc`], [`topological_sort`] use **directed** adjacency (only
//!   out-neighbors).
//!
//! ## Empty-projection contract (spec 16 §E09)
//!
//! All algorithms accept an empty projection and return an empty result
//! without error: `wcc(empty) == vec![]`, `topological_sort(empty) ==
//! Ok(vec![])`, etc.

mod articulation;
mod components;
mod topo;

pub use articulation::{articulation_points, bridges};
pub use components::{scc, scc_count, wcc, wcc_count};
pub use topo::{TopoSortError, topological_sort};

/// Sentinel value used by DFS state arrays to mark unvisited rows.
///
/// Set to `u32::MAX` so any valid row index (`< proj.max_row() + 1 ≤ u32::MAX`)
/// is distinguishable from "not yet discovered."
pub(crate) const SENTINEL: u32 = u32::MAX;
