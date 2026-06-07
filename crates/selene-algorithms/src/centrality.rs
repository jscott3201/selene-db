//! Centrality algorithms: PageRank, Brandes' betweenness.
//!
//! Betweenness operates on the directed view of a [`crate::GraphProjection`].
//! PageRank defaults to the same natural directed view and can opt into
//! reverse or undirected traversal for graph-retrieval workloads. State arrays
//! are sized by live-node count via the structural row-index helper per §E20 —
//! never by `max_row + 1` (donor pattern that breaks on filtered projections).
//!
//! ## Result shape (spec 16 §E21)
//!
//! Both algorithms return `Vec<(NodeId, f64)>` sorted **DESC by score with
//! NodeId ASC tie-break**. This is asymmetric to structural/pathfinding
//! outputs (which sort ASC by NodeId) — centrality is a scoring function
//! producing a ranking, not a set-position output. The sort comparator uses
//! `f64::total_cmp` (NaN-soundness) chained with NodeId ascending per
//! `feedback_dijkstra_tie_break_needs_both_rules` from PR #59.

mod betweenness;
mod pagerank;

pub use betweenness::{BetweennessConfig, betweenness, betweenness_with_checker};
pub use pagerank::{PageRankConfig, PageRankOrientation, pagerank, pagerank_with_checker};
