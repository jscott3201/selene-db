//! Native `algo.*` procedure definitions and dispatch.
//!
//! This module replaces the historical procedure-pack external-adapter path with
//! a concrete, in-crate binding: the 19 `algo.*` procedures call the
//! `selene-algorithms` native API (the `*_with_checker` algorithm functions plus
//! the projection catalog) directly, with **no** `ExternalGraphProcedure`
//! indirection. Argument coercion (`args`), error translation (`error`),
//! parallelism parsing (`parallel`), and per-`GraphId` projection state (`state`)
//! are ported from the pack so the native procedure metadata, row shapes, and
//! error strings preserve the historical algorithm contract.
//!
//! Every procedure is read-only graph-tier ([`ProcedureTier::Graph`] +
//! [`ProcedureMutability::Read`]) and increments [`metrics::ALGORITHM_RUNS_TOTAL`]
//! on a successful run (the historical `MeasuredGraphProcedure` behavior).

mod args;
mod centrality;
mod community;
mod error;
mod meta;
mod parallel;
mod pathfinding;
mod projection;
mod state;
mod structural;

use selene_core::{CancellationChecker, GraphId, Value, metrics};
use selene_graph::SeleneGraph;

use crate::procedure_registry::{ProcedureError, ProcedureHandle};
use crate::{
    GraphContext, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureParameter, ProcedureResult, ProcedureSignature, ProcedureTier,
};

pub(crate) use state::AlgorithmCatalogs;

/// One native algorithm procedure, identified by its dispatch kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlgoKind {
    ProjectionBuild,
    ProjectionGet,
    ProjectionDrop,
    ProjectionList,
    PageRank,
    Betweenness,
    LabelPropagation,
    Louvain,
    TriangleCount,
    Wcc,
    Scc,
    WccCount,
    SccCount,
    TopologicalSort,
    ArticulationPoints,
    Bridges,
    Dijkstra,
    Sssp,
    Apsp,
}

/// Static descriptor binding a canonical procedure name to its dispatch kind.
pub(super) struct AlgoSpec {
    /// Canonical multipart CALL name, e.g. `["algo", "pagerank"]`.
    pub(super) name: &'static [&'static str],
    /// Human-readable summary used by `SHOW PROCEDURES`.
    pub(super) description: &'static str,
    /// Dispatch kind.
    pub(super) kind: AlgoKind,
}

/// The 19 native `algo.*` procedures, in the historical pack registration order.
pub(super) const ALGO_SPECS: [AlgoSpec; 19] = [
    AlgoSpec {
        name: &["algo", "projection_build"],
        description: "Build an algorithm projection.",
        kind: AlgoKind::ProjectionBuild,
    },
    AlgoSpec {
        name: &["algo", "projection_get"],
        description: "Inspect one algorithm projection.",
        kind: AlgoKind::ProjectionGet,
    },
    AlgoSpec {
        name: &["algo", "projection_drop"],
        description: "Drop an algorithm projection.",
        kind: AlgoKind::ProjectionDrop,
    },
    AlgoSpec {
        name: &["algo", "projection_list"],
        description: "List algorithm projections.",
        kind: AlgoKind::ProjectionList,
    },
    AlgoSpec {
        name: &["algo", "pagerank"],
        description: "Compute PageRank scores for a projection.",
        kind: AlgoKind::PageRank,
    },
    AlgoSpec {
        name: &["algo", "betweenness"],
        description: "Compute betweenness centrality scores.",
        kind: AlgoKind::Betweenness,
    },
    AlgoSpec {
        name: &["algo", "label_propagation"],
        description: "Run label propagation community detection.",
        kind: AlgoKind::LabelPropagation,
    },
    AlgoSpec {
        name: &["algo", "louvain"],
        description: "Run Louvain community detection.",
        kind: AlgoKind::Louvain,
    },
    AlgoSpec {
        name: &["algo", "triangle_count"],
        description: "Count triangles per node in a projection.",
        kind: AlgoKind::TriangleCount,
    },
    AlgoSpec {
        name: &["algo", "wcc"],
        description: "Compute weakly connected components.",
        kind: AlgoKind::Wcc,
    },
    AlgoSpec {
        name: &["algo", "scc"],
        description: "Compute strongly connected components.",
        kind: AlgoKind::Scc,
    },
    AlgoSpec {
        name: &["algo", "wcc_count"],
        description: "Count weakly connected components.",
        kind: AlgoKind::WccCount,
    },
    AlgoSpec {
        name: &["algo", "scc_count"],
        description: "Count strongly connected components.",
        kind: AlgoKind::SccCount,
    },
    AlgoSpec {
        name: &["algo", "topological_sort"],
        description: "Return a topological ordering for a projection.",
        kind: AlgoKind::TopologicalSort,
    },
    AlgoSpec {
        name: &["algo", "articulation_points"],
        description: "Find articulation points in a projection.",
        kind: AlgoKind::ArticulationPoints,
    },
    AlgoSpec {
        name: &["algo", "bridges"],
        description: "Find bridge edges in a projection.",
        kind: AlgoKind::Bridges,
    },
    AlgoSpec {
        name: &["algo", "dijkstra"],
        description: "Find the shortest path between two nodes.",
        kind: AlgoKind::Dijkstra,
    },
    AlgoSpec {
        name: &["algo", "sssp"],
        description: "Compute single-source shortest paths.",
        kind: AlgoKind::Sssp,
    },
    AlgoSpec {
        name: &["algo", "apsp"],
        description: "Compute all-pairs shortest paths.",
        kind: AlgoKind::Apsp,
    },
];

impl AlgoKind {
    /// Build the planner-visible metadata for this procedure.
    ///
    /// `handle` is the registry-assigned opaque dispatch handle; `description`
    /// is the matching [`AlgoSpec::description`]. Tier/mutability are fixed:
    /// every algorithm procedure is read-only graph-tier.
    pub(super) fn metadata(
        self,
        handle: ProcedureHandle,
        description: &'static str,
    ) -> ProcedureMetadata {
        ProcedureMetadata::new(
            handle,
            ProcedureSignature::new(self.signature()),
            ProcedureOutputSchema {
                columns: self.output_columns(),
            },
            ProcedureTier::Graph,
            ProcedureMutability::Read,
        )
        .with_description(description)
    }

    fn signature(self) -> Vec<ProcedureParameter> {
        match self {
            Self::ProjectionBuild => projection::build_signature(),
            Self::ProjectionGet => projection::get_signature(),
            Self::ProjectionDrop => projection::drop_signature(),
            Self::ProjectionList => projection::list_signature(),
            Self::PageRank => centrality::pagerank_signature(),
            Self::Betweenness => centrality::betweenness_signature(),
            Self::LabelPropagation => community::label_propagation_signature(),
            Self::Louvain => community::louvain_signature(),
            Self::TriangleCount => community::triangle_count_signature(),
            Self::Wcc
            | Self::Scc
            | Self::WccCount
            | Self::SccCount
            | Self::TopologicalSort
            | Self::ArticulationPoints
            | Self::Bridges => structural::projection_signature(),
            Self::Dijkstra => pathfinding::dijkstra_signature(),
            Self::Sssp => pathfinding::sssp_signature(),
            Self::Apsp => pathfinding::apsp_signature(),
        }
    }

    fn output_columns(self) -> Vec<ProcedureOutputColumn> {
        match self {
            Self::ProjectionBuild | Self::ProjectionDrop => projection::empty_columns(),
            Self::ProjectionGet | Self::ProjectionList => projection::projection_output_columns(),
            Self::PageRank | Self::Betweenness => centrality::node_score_columns(),
            Self::LabelPropagation => community::node_community_columns(),
            Self::Louvain => community::louvain_columns(),
            Self::TriangleCount => community::triangle_count_columns(),
            Self::Wcc | Self::Scc => structural::node_component_columns(),
            Self::WccCount | Self::SccCount => structural::count_columns(),
            Self::TopologicalSort => structural::topo_columns(),
            Self::ArticulationPoints => structural::node_id_columns(),
            Self::Bridges => structural::bridges_columns(),
            Self::Dijkstra => pathfinding::dijkstra_columns(),
            Self::Sssp => pathfinding::sssp_columns(),
            Self::Apsp => pathfinding::apsp_columns(),
        }
    }

    /// Execute this procedure against a read-only graph context.
    ///
    /// Increments [`metrics::ALGORITHM_RUNS_TOTAL`] on success, matching the
    /// historical `MeasuredGraphProcedure` wrapper.
    pub(super) fn execute(
        self,
        catalogs: &AlgorithmCatalogs,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let snapshot = ctx.snapshot();
        let checker = ctx.cancellation_checker();
        let result = self.run(catalogs, snapshot, args, checker);
        if result.is_ok() {
            metrics::counter_inc(metrics::ALGORITHM_RUNS_TOTAL);
        }
        result
    }

    fn run(
        self,
        catalogs: &AlgorithmCatalogs,
        snapshot: &SeleneGraph,
        args: &[Value],
        checker: CancellationChecker<'_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        match self {
            // Projection lifecycle takes the snapshot directly (no checker).
            Self::ProjectionBuild => projection::build(catalogs, snapshot, args),
            Self::ProjectionGet => projection::get(catalogs, snapshot, args),
            Self::ProjectionDrop => projection::drop_projection(catalogs, snapshot, args),
            Self::ProjectionList => projection::list(catalogs, snapshot, args),
            Self::PageRank => centrality::pagerank(catalogs, snapshot, args, checker),
            Self::Betweenness => centrality::betweenness(catalogs, snapshot, args, checker),
            Self::LabelPropagation => {
                community::label_propagation(catalogs, snapshot, args, checker)
            }
            Self::Louvain => community::louvain(catalogs, snapshot, args, checker),
            Self::TriangleCount => community::triangle_count(catalogs, snapshot, args, checker),
            Self::Wcc => structural::wcc(catalogs, snapshot, args, checker),
            Self::Scc => structural::scc(catalogs, snapshot, args, checker),
            Self::WccCount => structural::wcc_count(catalogs, snapshot, args, checker),
            Self::SccCount => structural::scc_count(catalogs, snapshot, args, checker),
            Self::TopologicalSort => {
                structural::topological_sort(catalogs, snapshot, args, checker)
            }
            Self::ArticulationPoints => {
                structural::articulation_points(catalogs, snapshot, args, checker)
            }
            Self::Bridges => structural::bridges(catalogs, snapshot, args, checker),
            Self::Dijkstra => pathfinding::dijkstra(catalogs, snapshot, args, checker),
            Self::Sssp => pathfinding::sssp(catalogs, snapshot, args, checker),
            Self::Apsp => pathfinding::apsp(catalogs, snapshot, args, checker),
        }
    }
}

/// Drop the projection catalog for a graph that is no longer live.
///
/// Exposed for the registry so an embedder can reclaim ephemeral projection
/// state when a graph is dropped (projections are derived state, never
/// persisted; a later `GraphId` reuse must not observe a stale projection).
pub(super) fn forget_graph(catalogs: &AlgorithmCatalogs, graph_id: GraphId) -> bool {
    catalogs.forget_graph(graph_id)
}
