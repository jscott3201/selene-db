//! [`GraphAlgorithms`] extension trait: ergonomic [`SeleneGraph`] methods over
//! the native algorithm free functions in [`super`].
//!
//! Implemented for [`SeleneGraph`] inside `selene-algorithms`, this trait adds
//! no dependency edge: `selene-algorithms` already depends on `selene-graph`,
//! and `selene-graph` does not depend on `selene-algorithms` (the storage ≠
//! algorithms concern boundary, spec 16 §E01). Each method forwards to the
//! matching free function, passing `self` as the snapshot.

use selene_core::NodeId;
use selene_graph::SeleneGraph;

use crate::catalog::ProjectionCatalog;
use crate::centrality::{BetweennessConfig, PageRankConfig};
use crate::community::TriangleCountConfig;
use crate::error::AlgorithmsError;
use crate::pathfinding::{ApspConfig, PathResult};
use crate::projection::ProjectionConfig;

use super::{
    ApiError, ProjectionInfo, apsp, articulation_points, betweenness, bridges, dijkstra,
    label_propagation, louvain, pagerank, projection_build, projection_get, projection_list, scc,
    scc_count, sssp, topological_sort, triangle_count, wcc, wcc_count,
};

/// Ergonomic [`SeleneGraph`] methods for the native algorithm API.
///
/// This trait gives `graph.pagerank(catalog, name, cfg)` ergonomics over the
/// free functions above. It is implemented for [`SeleneGraph`] inside
/// `selene-algorithms`, so it adds **no** dependency edge: `selene-algorithms`
/// already depends on `selene-graph`, and `selene-graph` does **not** depend on
/// `selene-algorithms` (the storage ≠ algorithms concern boundary, Hard
/// Rule / spec 16 §E01). Each method forwards to the matching free function,
/// passing `self` as the snapshot.
///
/// The caller still owns the [`ProjectionCatalog`] (projections are ephemeral,
/// per-`GraphId` engine-internal state) and passes it explicitly. To run with
/// cooperative cancellation, call the free `*_with_checker` functions directly.
pub trait GraphAlgorithms {
    /// See [`projection_build`].
    ///
    /// # Errors
    /// Returns [`AlgorithmsError`] if projection construction fails.
    fn projection_build(
        &self,
        catalog: &ProjectionCatalog,
        config: &ProjectionConfig,
    ) -> Result<(usize, usize), AlgorithmsError>;

    /// See [`projection_get`].
    ///
    /// # Errors
    /// Returns [`AlgorithmsError`] if the projection cannot be resolved.
    fn projection_get(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<ProjectionInfo, AlgorithmsError>;

    /// See [`projection_list`].
    ///
    /// # Errors
    /// Returns [`AlgorithmsError`] if any projection fails to refresh.
    fn projection_list(
        &self,
        catalog: &ProjectionCatalog,
    ) -> Result<Vec<ProjectionInfo>, AlgorithmsError>;

    /// See [`pagerank`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn pagerank(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: PageRankConfig,
    ) -> Result<Vec<(NodeId, f64)>, ApiError>;

    /// See [`betweenness`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn betweenness(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: BetweennessConfig,
    ) -> Result<Vec<(NodeId, f64)>, ApiError>;

    /// See [`label_propagation`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn label_propagation(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        max_iter: usize,
    ) -> Result<Vec<(NodeId, u64)>, ApiError>;

    /// See [`louvain`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn louvain(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        max_iter: usize,
    ) -> Result<Vec<(NodeId, u64, u32)>, ApiError>;

    /// See [`triangle_count`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn triangle_count(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: TriangleCountConfig,
    ) -> Result<Vec<(NodeId, usize)>, ApiError>;

    /// See [`wcc`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn wcc(&self, catalog: &ProjectionCatalog, name: &str) -> Result<Vec<(NodeId, u64)>, ApiError>;

    /// See [`scc`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn scc(&self, catalog: &ProjectionCatalog, name: &str) -> Result<Vec<(NodeId, u64)>, ApiError>;

    /// See [`wcc_count`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn wcc_count(&self, catalog: &ProjectionCatalog, name: &str) -> Result<usize, ApiError>;

    /// See [`scc_count`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn scc_count(&self, catalog: &ProjectionCatalog, name: &str) -> Result<usize, ApiError>;

    /// See [`topological_sort`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure or a directed cycle.
    fn topological_sort(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<Vec<(NodeId, usize)>, ApiError>;

    /// See [`articulation_points`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn articulation_points(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<Vec<NodeId>, ApiError>;

    /// See [`bridges`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure.
    fn bridges(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<Vec<(NodeId, NodeId)>, ApiError>;

    /// See [`dijkstra`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure or a bad weight.
    fn dijkstra(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        from: NodeId,
        to: NodeId,
    ) -> Result<Option<PathResult>, ApiError>;

    /// See [`sssp`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure or a bad weight.
    fn sssp(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        source: NodeId,
    ) -> Result<Vec<(NodeId, f64)>, ApiError>;

    /// See [`apsp`].
    ///
    /// # Errors
    /// Returns [`ApiError`] on projection resolution failure, a bad weight, or
    /// a node count exceeding `config.max_nodes`.
    fn apsp(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: ApspConfig,
    ) -> Result<Vec<(NodeId, NodeId, f64)>, ApiError>;
}

impl GraphAlgorithms for SeleneGraph {
    fn projection_build(
        &self,
        catalog: &ProjectionCatalog,
        config: &ProjectionConfig,
    ) -> Result<(usize, usize), AlgorithmsError> {
        projection_build(catalog, self, config)
    }

    fn projection_get(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<ProjectionInfo, AlgorithmsError> {
        projection_get(catalog, self, name)
    }

    fn projection_list(
        &self,
        catalog: &ProjectionCatalog,
    ) -> Result<Vec<ProjectionInfo>, AlgorithmsError> {
        projection_list(catalog, self)
    }

    fn pagerank(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: PageRankConfig,
    ) -> Result<Vec<(NodeId, f64)>, ApiError> {
        pagerank(catalog, self, name, config)
    }

    fn betweenness(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: BetweennessConfig,
    ) -> Result<Vec<(NodeId, f64)>, ApiError> {
        betweenness(catalog, self, name, config)
    }

    fn label_propagation(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        max_iter: usize,
    ) -> Result<Vec<(NodeId, u64)>, ApiError> {
        label_propagation(catalog, self, name, max_iter)
    }

    fn louvain(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        max_iter: usize,
    ) -> Result<Vec<(NodeId, u64, u32)>, ApiError> {
        louvain(catalog, self, name, max_iter)
    }

    fn triangle_count(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: TriangleCountConfig,
    ) -> Result<Vec<(NodeId, usize)>, ApiError> {
        triangle_count(catalog, self, name, config)
    }

    fn wcc(&self, catalog: &ProjectionCatalog, name: &str) -> Result<Vec<(NodeId, u64)>, ApiError> {
        wcc(catalog, self, name)
    }

    fn scc(&self, catalog: &ProjectionCatalog, name: &str) -> Result<Vec<(NodeId, u64)>, ApiError> {
        scc(catalog, self, name)
    }

    fn wcc_count(&self, catalog: &ProjectionCatalog, name: &str) -> Result<usize, ApiError> {
        wcc_count(catalog, self, name)
    }

    fn scc_count(&self, catalog: &ProjectionCatalog, name: &str) -> Result<usize, ApiError> {
        scc_count(catalog, self, name)
    }

    fn topological_sort(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<Vec<(NodeId, usize)>, ApiError> {
        topological_sort(catalog, self, name)
    }

    fn articulation_points(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<Vec<NodeId>, ApiError> {
        articulation_points(catalog, self, name)
    }

    fn bridges(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
    ) -> Result<Vec<(NodeId, NodeId)>, ApiError> {
        bridges(catalog, self, name)
    }

    fn dijkstra(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        from: NodeId,
        to: NodeId,
    ) -> Result<Option<PathResult>, ApiError> {
        dijkstra(catalog, self, name, from, to)
    }

    fn sssp(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        source: NodeId,
    ) -> Result<Vec<(NodeId, f64)>, ApiError> {
        sssp(catalog, self, name, source)
    }

    fn apsp(
        &self,
        catalog: &ProjectionCatalog,
        name: &str,
        config: ApspConfig,
    ) -> Result<Vec<(NodeId, NodeId, f64)>, ApiError> {
        apsp(catalog, self, name, config)
    }
}
