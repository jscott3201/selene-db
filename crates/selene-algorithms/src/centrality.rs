//! Centrality algorithms: PageRank, Brandes' betweenness.
//!
//! Both surfaces operate on the directed view of a [`crate::GraphProjection`]
//! (only `out_neighbors`) per spec 16 §E19. State arrays are sized by
//! live-node count via [`crate::structural::RowIndex`] per §E20 — never by
//! `max_row + 1` (donor pattern that breaks on filtered projections; see
//! BRIEF-52 PR #58 lesson `feedback_donor_pattern_invariant_check`).
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

pub use betweenness::betweenness;
pub use pagerank::{PageRankConfig, pagerank};
