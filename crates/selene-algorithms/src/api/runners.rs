//! Native algorithm runner free functions.
//!
//! Each surface resolves a named projection from the caller-owned
//! [`ProjectionCatalog`] (refreshing it against the snapshot generation) and
//! then runs the underlying algorithm, returning the algorithm's typed result.
//! Every runner has a convenience variant (disabled cancellation checker) and a
//! `*_with_checker` variant. Projection resolution + lock discipline live in
//! [`super::with_projection`]; see the [`super`] module docs.
//!
//! Algorithm functions are referenced via fully-qualified `crate::` paths so the
//! facade can reuse the canonical `*_with_checker` names without shadowing the
//! underlying algorithm functions.
//!
//! Delegation shape: every convenience runner forwards to its `*_with_checker`
//! sibling with [`CancellationChecker::disabled`]; the projection-resolution
//! body lives once in the `*_with_checker` variant. There is no separate
//! private `*_checked` helper tier.

use selene_core::{CancellationChecker, NodeId};
use selene_graph::SeleneGraph;

use crate::catalog::ProjectionCatalog;
use crate::centrality::{BetweennessConfig, PageRankConfig};
use crate::community::TriangleCountConfig;
use crate::pathfinding::{ApspConfig, PathResult};

use super::{ApiError, with_projection};

/// Run PageRank over the named projection. See [`crate::pagerank`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
/// PageRank itself is infallible aside from cancellation.
pub fn pagerank(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: PageRankConfig,
) -> Result<Vec<(NodeId, f64)>, ApiError> {
    pagerank_with_checker(
        catalog,
        snapshot,
        name,
        config,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`pagerank`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn pagerank_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: PageRankConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::centrality::pagerank_with_checker(
            projection, config, checker,
        )?)
    })
}

/// Run betweenness centrality over the named projection. See
/// [`crate::betweenness`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn betweenness(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: BetweennessConfig,
) -> Result<Vec<(NodeId, f64)>, ApiError> {
    betweenness_with_checker(
        catalog,
        snapshot,
        name,
        config,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`betweenness`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn betweenness_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: BetweennessConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::centrality::betweenness_with_checker(
            projection, config, checker,
        )?)
    })
}

// ---------------------------------------------------------------------------
// Community
// ---------------------------------------------------------------------------

/// Run label propagation over the named projection. See
/// [`crate::label_propagation`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn label_propagation(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    max_iter: usize,
) -> Result<Vec<(NodeId, u64)>, ApiError> {
    label_propagation_with_checker(
        catalog,
        snapshot,
        name,
        max_iter,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`label_propagation`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn label_propagation_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    max_iter: usize,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, u64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::community::label_propagation_with_checker(
            projection, max_iter, checker,
        )?)
    })
}

/// Run Louvain community detection over the named projection. See
/// [`crate::louvain`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn louvain(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    max_iter: usize,
) -> Result<Vec<(NodeId, u64, u32)>, ApiError> {
    louvain_with_checker(
        catalog,
        snapshot,
        name,
        max_iter,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`louvain`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn louvain_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    max_iter: usize,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, u64, u32)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::community::louvain_with_checker(
            projection, max_iter, checker,
        )?)
    })
}

/// Count triangles per node over the named projection. See
/// [`crate::triangle_count`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn triangle_count(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: TriangleCountConfig,
) -> Result<Vec<(NodeId, usize)>, ApiError> {
    triangle_count_with_checker(
        catalog,
        snapshot,
        name,
        config,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`triangle_count`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn triangle_count_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: TriangleCountConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, usize)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::community::triangle_count_with_checker(
            projection, config, checker,
        )?)
    })
}

// ---------------------------------------------------------------------------
// Structural
// ---------------------------------------------------------------------------

/// Compute weakly connected components over the named projection. See
/// [`crate::wcc`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn wcc(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<Vec<(NodeId, u64)>, ApiError> {
    wcc_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`wcc`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn wcc_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, u64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::wcc_with_checker(projection, checker)?)
    })
}

/// Compute strongly connected components over the named projection. See
/// [`crate::scc`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn scc(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<Vec<(NodeId, u64)>, ApiError> {
    scc_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`scc`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn scc_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, u64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::scc_with_checker(projection, checker)?)
    })
}

/// Count weakly connected components over the named projection. See
/// [`crate::wcc_count`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn wcc_count(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<usize, ApiError> {
    wcc_count_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`wcc_count`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn wcc_count_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<usize, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::wcc_count_with_checker(
            projection, checker,
        )?)
    })
}

/// Count strongly connected components over the named projection. See
/// [`crate::scc_count`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn scc_count(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<usize, ApiError> {
    scc_count_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`scc_count`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn scc_count_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<usize, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::scc_count_with_checker(
            projection, checker,
        )?)
    })
}

/// Topologically sort the named projection. See [`crate::topological_sort`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved, or
/// [`ApiError::TopoSort`] if the projection contains a directed cycle.
pub fn topological_sort(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<Vec<(NodeId, usize)>, ApiError> {
    topological_sort_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`topological_sort`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure, or
/// [`ApiError::TopoSort`] on a directed cycle / cancellation.
pub fn topological_sort_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, usize)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::topological_sort_with_checker(
            projection, checker,
        )?)
    })
}

/// Find articulation points in the named projection. See
/// [`crate::articulation_points`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn articulation_points(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<Vec<NodeId>, ApiError> {
    articulation_points_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`articulation_points`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn articulation_points_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<Vec<NodeId>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::articulation_points_with_checker(
            projection, checker,
        )?)
    })
}

/// Find bridge edges in the named projection. See [`crate::bridges`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved.
pub fn bridges(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
) -> Result<Vec<(NodeId, NodeId)>, ApiError> {
    bridges_with_checker(catalog, snapshot, name, CancellationChecker::disabled())
}

/// Cancellable [`bridges`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure or
/// [`ApiError::Aborted`] on cancellation / timeout.
pub fn bridges_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, NodeId)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::structural::bridges_with_checker(
            projection, checker,
        )?)
    })
}

// ---------------------------------------------------------------------------
// Pathfinding
// ---------------------------------------------------------------------------

/// Shortest path between two nodes over the named projection. See
/// [`crate::dijkstra`].
///
/// Returns `Ok(None)` when `from`/`to` is absent or unreachable.
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved, or
/// [`ApiError::Pathfinding`] on a traversed negative / NaN edge weight.
pub fn dijkstra(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    from: NodeId,
    to: NodeId,
) -> Result<Option<PathResult>, ApiError> {
    dijkstra_with_checker(
        catalog,
        snapshot,
        name,
        from,
        to,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`dijkstra`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure, or
/// [`ApiError::Pathfinding`] on a bad edge weight / cancellation.
pub fn dijkstra_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    from: NodeId,
    to: NodeId,
    checker: CancellationChecker<'_>,
) -> Result<Option<PathResult>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::pathfinding::dijkstra_with_checker(
            projection, from, to, checker,
        )?)
    })
}

/// Single-source shortest paths over the named projection. See [`crate::sssp`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved, or
/// [`ApiError::Pathfinding`] on a traversed negative / NaN edge weight.
pub fn sssp(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    source: NodeId,
) -> Result<Vec<(NodeId, f64)>, ApiError> {
    sssp_with_checker(
        catalog,
        snapshot,
        name,
        source,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`sssp`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure, or
/// [`ApiError::Pathfinding`] on a bad edge weight / cancellation.
pub fn sssp_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    source: NodeId,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::pathfinding::sssp_with_checker(
            projection, source, checker,
        )?)
    })
}

/// All-pairs shortest paths over the named projection. See [`crate::apsp`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] if the projection cannot be resolved, or
/// [`ApiError::Pathfinding`] on a bad edge weight or a node count exceeding
/// `config.max_nodes`.
pub fn apsp(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: ApspConfig,
) -> Result<Vec<(NodeId, NodeId, f64)>, ApiError> {
    apsp_with_checker(
        catalog,
        snapshot,
        name,
        config,
        CancellationChecker::disabled(),
    )
}

/// Cancellable [`apsp`].
///
/// # Errors
///
/// Returns [`ApiError::Projection`] on projection resolution failure, or
/// [`ApiError::Pathfinding`] on a bad edge weight / size guard / cancellation.
pub fn apsp_with_checker(
    catalog: &ProjectionCatalog,
    snapshot: &SeleneGraph,
    name: &str,
    config: ApspConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, NodeId, f64)>, ApiError> {
    with_projection(catalog, snapshot, name, |projection| {
        Ok(crate::pathfinding::apsp_with_checker(
            projection, config, checker,
        )?)
    })
}
