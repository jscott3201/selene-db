//! Native Rust algorithm API facade.
//!
//! This module is the first-class, registry-free entry point for running graph
//! algorithms. Every surface follows the same "resolve projection → run
//! algorithm → return typed result" shape that the GQL `CALL algo.*` procedures
//! drive, but exposed as plain free functions (and, for ergonomics, as methods
//! on [`SeleneGraph`] via the [`GraphAlgorithms`] extension trait). No GQL,
//! procedure-registry, or `Value` machinery is involved — the facade is part of
//! `selene-algorithms` and depends only on `selene-core` + `selene-graph`.
//!
//! # Projection lifecycle
//!
//! Algorithms run over a [`GraphProjection`] — a frozen, CSR-backed view of the
//! live graph. Projections are named and cached in a caller-owned
//! [`ProjectionCatalog`]. The lifecycle helpers ([`projection_build`],
//! [`projection_get`], [`projection_drop`], [`projection_list`]) manage the
//! catalog; each algorithm runner ([`pagerank`], [`wcc`], …) resolves a
//! projection by name, refreshes it against the snapshot generation, then runs.
//!
//! ## Catalog ownership
//!
//! The catalog is **caller-owned and ephemeral** — projections are derived
//! state, never persisted and never part of snapshot/recovery. The embedder
//! (e.g. the GQL built-in procedure registry) holds one [`ProjectionCatalog`]
//! per `GraphId` and clears it when the graph is dropped. This module does not
//! own any catalog state.
//!
//! ## Lock discipline (spec 16 §3 E07)
//!
//! Each runner calls [`ProjectionCatalog::ensure_fresh`] (which may take the
//! catalog write lock to rebuild a stale projection) and then
//! [`ProjectionCatalog::get`] (which holds a read guard for the returned
//! [`crate::ProjectionRef`]). The algorithm runs while the read guard is held;
//! the guard is dropped before returning. Callers MUST NOT hold the graph write
//! lock across these calls, and MUST NOT call a catalog mutation
//! (`drop_projection`, `project`) while a [`crate::ProjectionRef`] is live.
//!
//! # Cancellation
//!
//! Every runner has a `*_with_checker` variant taking a
//! [`selene_core::CancellationChecker`]; the convenience variant uses
//! [`selene_core::CancellationChecker::disabled`]. The checker is threaded into
//! the underlying algorithm so a long run observes cooperative cancellation /
//! deadline timeout at the algorithm's checkpoints.

// The algorithm runner free functions live in `runners`; the ergonomic
// `SeleneGraph` extension trait lives in `graph_ext`. Both are re-exported below
// so the public surface is `selene_algorithms::api::*`. This module owns the
// shared `ApiError` / `ProjectionInfo` types, the `with_projection` resolution
// helper, and the projection-lifecycle free functions.
use selene_graph::SeleneGraph;

use crate::catalog::ProjectionCatalog;
use crate::error::{AlgorithmAborted, AlgorithmsError};
use crate::pathfinding::PathfindingError;
use crate::projection::{GraphProjection, ProjectionConfig};
use crate::structural::TopoSortError;

/// Error returned by the native algorithm facade.
///
/// The facade unifies the three failure modes a "resolve projection → run
/// algorithm" call can hit, so every runner has a single `Result` type:
///
/// - [`ApiError::Projection`] — the named projection is missing or could not be
///   (re)built (an [`AlgorithmsError`]).
/// - [`ApiError::Aborted`] — the algorithm observed cooperative cancellation or
///   a deadline timeout ([`AlgorithmAborted`]).
/// - [`ApiError::Pathfinding`] — a pathfinding surface refused an edge weight or
///   exceeded its node-count guard ([`PathfindingError`]).
/// - [`ApiError::TopoSort`] — [`topological_sort`] found a directed cycle (or
///   was aborted).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    /// The named projection is missing or could not be (re)built.
    #[error(transparent)]
    Projection(#[from] AlgorithmsError),
    /// The algorithm was cooperatively cancelled or timed out.
    #[error(transparent)]
    Aborted(#[from] AlgorithmAborted),
    /// A pathfinding surface refused an edge weight or exceeded its node guard.
    #[error(transparent)]
    Pathfinding(#[from] PathfindingError),
    /// `topological_sort` found a directed cycle, or was aborted.
    #[error(transparent)]
    TopoSort(#[from] TopoSortError),
}

/// Immutable snapshot of one projection's identity and size.
///
/// Returned by [`projection_get`] / [`projection_list`] so callers can render
/// projection metadata without holding the catalog read guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionInfo {
    /// Projection name (the catalog key).
    pub name: String,
    /// Graph generation the projection was last built against.
    pub generation: u64,
    /// Number of nodes in the projection.
    pub node_count: u64,
    /// Number of outgoing edges in the projection.
    pub edge_count: u64,
}

impl ProjectionInfo {
    fn from_projection(projection: &GraphProjection) -> Self {
        Self {
            name: projection.name().to_owned(),
            generation: projection.generation(),
            node_count: projection.node_count() as u64,
            edge_count: projection.edge_count() as u64,
        }
    }
}

// ---------------------------------------------------------------------------
// Projection lifecycle
// ---------------------------------------------------------------------------

/// Build (or rebuild) a named projection in `catalog` from `config`.
///
/// Returns `(node_count, edge_count)`. Overwrites any existing projection of
/// the same name in one atomic write-lock acquisition (see
/// [`ProjectionCatalog::project`]). Catalog projections are always unscoped:
/// `config` is the complete recipe a stale-rebuild reproduces (spec 16 §3
/// E06); scoped views go through [`GraphProjection::build`] directly.
///
/// # Errors
///
/// Returns [`AlgorithmsError`] if projection construction fails.
pub fn projection_build(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    config: &ProjectionConfig,
) -> Result<(usize, usize), AlgorithmsError> {
    catalog.project(snapshot, config)
}

/// Resolve and refresh a single named projection, returning its
/// [`ProjectionInfo`].
///
/// Refreshes the projection against `snapshot`'s generation before reading, so
/// the returned counts reflect the current graph.
///
/// # Errors
///
/// Returns [`AlgorithmsError::NoSuchProjection`] when `name` is not registered,
/// or another [`AlgorithmsError`] if a stale projection fails to rebuild.
pub fn projection_get(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<ProjectionInfo, AlgorithmsError> {
    catalog.ensure_fresh(snapshot, name)?;
    let projection = catalog
        .get(name)
        .ok_or_else(|| AlgorithmsError::NoSuchProjection {
            name: name.to_owned(),
        })?;
    Ok(ProjectionInfo::from_projection(projection.projection()))
}

/// Drop a named projection from `catalog`. Returns `true` if it existed.
pub fn projection_drop(catalog: &ProjectionCatalog, name: &str) -> bool {
    catalog.drop_projection(name)
}

/// List all projections in `catalog`, refreshed against `snapshot`, sorted ASC
/// by name.
///
/// # Errors
///
/// Returns [`AlgorithmsError`] if any registered projection fails to refresh.
pub fn projection_list(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
) -> Result<Vec<ProjectionInfo>, AlgorithmsError> {
    let mut names = catalog.names();
    names.sort();
    names
        .iter()
        .map(|name| projection_get(catalog, snapshot, name))
        .collect()
}

/// Run `f` against a fresh, read-locked view of the named projection.
///
/// This mirrors the GQL adapter's `with_algorithm_projection`: `ensure_fresh`
/// (rebuild-if-stale) then `get` (read-locked [`crate::ProjectionRef`]), running `f`
/// while the read guard is held and dropping it on return. Keeping it in one
/// place guarantees every runner observes identical projection-resolution
/// semantics and lock ordering.
pub(super) fn with_projection<R>(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    f: impl FnOnce(&GraphProjection) -> Result<R, ApiError>,
) -> Result<R, ApiError> {
    catalog.ensure_fresh(snapshot, name)?;
    let projection = catalog
        .get(name)
        .ok_or_else(|| AlgorithmsError::NoSuchProjection {
            name: name.to_owned(),
        })?;
    f(projection.projection())
}

mod graph_ext;
mod runners;

#[cfg(test)]
mod tests;

pub use graph_ext::GraphAlgorithms;
pub use runners::{
    apsp, apsp_with_checker, articulation_points, articulation_points_with_checker, betweenness,
    betweenness_with_checker, bridges, bridges_with_checker, dijkstra, dijkstra_with_checker,
    label_propagation, label_propagation_with_checker, louvain, louvain_with_checker, pagerank,
    pagerank_with_checker, scc, scc_count, scc_count_with_checker, scc_with_checker, sssp,
    sssp_with_checker, topological_sort, topological_sort_with_checker, triangle_count,
    triangle_count_with_checker, wcc, wcc_count, wcc_count_with_checker, wcc_with_checker,
};
