//! Community detection: label propagation, Louvain modularity, triangle count.
//!
//! All three surfaces operate on the **undirected** view of a
//! [`crate::GraphProjection`] (union of `out_neighbors` and `in_neighbors`) per
//! spec 16 §E25. State arrays are sized by live-node count via
//! [`crate::structural::RowIndex`] per §E26 — never by `max_row + 1` (donor
//! pattern that breaks on filtered projections; see BRIEF-52 PR #58 lesson
//! `feedback_donor_pattern_invariant_check`).
//!
//! ## Multiplicity (spec 16 §E25)
//!
//! Parallel edges contribute via multiplicity to `label_propagation`'s
//! label counts and `louvain`'s modularity gain (donor pattern: iterate both
//! `out_neighbors` and `in_neighbors` without deduping). For
//! `triangle_count`, parallel edges collapse to a single neighbor via
//! `sort_unstable() + dedup()`; a triangle is defined as 3 distinct mutually-
//! connected nodes (§E29).
//!
//! ## Result shapes (spec 16 §E27)
//!
//! - `label_propagation`: `Vec<(NodeId, u64)>` sorted **ASC by NodeId** —
//!   community IDs are categorical (smallest stable label per Raghavan).
//! - `louvain`: `Vec<(NodeId, u64, u32)>` sorted **ASC by NodeId** — the `u32`
//!   is `level`, reserved at `0` in v1.0 for forward-compat with hierarchical
//!   Louvain (v1.x).
//! - `triangle_count`: `Vec<(NodeId, usize)>` sorted **DESC by count** with
//!   **NodeId ASC** tie-break.
//!
//! ## Determinism (spec 16 §E30)
//!
//! `label_propagation` tie-breaks equal-frequency labels by smallest ID.
//! `louvain` iterates candidate communities in **sorted order by community ID**
//! (defuses HashMap-iteration-order non-determinism per the
//! `feedback_pagerank_dangling_bulk_apply` donor-skepticism lesson) and only
//! moves on strictly-positive modularity gain.

mod label_propagation;
mod louvain;
mod triangle_count;

pub use label_propagation::label_propagation;
pub use louvain::louvain;
pub use triangle_count::triangle_count;
