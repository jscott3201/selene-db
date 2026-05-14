//! Pathfinding algorithms: Dijkstra, single-source shortest path (SSSP),
//! all-pairs shortest path (APSP).
//!
//! All algorithms operate on the directed view of a [`crate::GraphProjection`]
//! (only `out_neighbors`) per spec 16 §E13. State arrays are sized by live-node
//! count via [`crate::structural::RowIndex`] (cross-imported) per §E14 — never
//! by `max_row + 1` (donor pattern that breaks on filtered projections; see
//! BRIEF-52 PR #58 Codex review).
//!
//! ## Weight contract (spec 16 §E15)
//!
//! Edge weights come from the projection (`ProjectionConfig::weight_property`)
//! per §E04: missing / non-numeric / null → `1.0`. Pathfinding additionally
//! validates **on traversal**:
//! - Negative finite → [`PathfindingError::NegativeWeight`].
//! - `NaN` → [`PathfindingError::NaNWeight`] (detected **before** the edge
//!   enters the heap, so `f64::total_cmp` never sees NaN ordering).
//! - `+f64::INFINITY` → accepted (the edge cannot improve any path).
//!
//! Validation is lazy — only edges actually traversed are checked. Projections
//! with bad weights at unreachable corners may still succeed.

mod apsp;
mod dijkstra;
mod error;
mod sssp;

pub use apsp::{ApspConfig, apsp};
pub use dijkstra::{PathResult, dijkstra};
pub use error::PathfindingError;
pub use sssp::sssp;
