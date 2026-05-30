//! Unit tests for the native algorithm API facade + extension trait.

use std::sync::Arc;

use selene_core::{
    CancellationChecker, CancellationToken, GraphId, IStr, LabelSet, NodeId, PropertyMap, intern,
};
use selene_graph::{SeleneGraph, SharedGraph};

use crate::api::{self, ApiError, GraphAlgorithms, ProjectionInfo};
use crate::catalog::ProjectionCatalog;
use crate::centrality::{BetweennessConfig, PageRankConfig};
use crate::community::TriangleCountConfig;
use crate::error::AlgorithmsError;
use crate::pathfinding::{ApspConfig, PathfindingError};
use crate::projection::ProjectionConfig;
use crate::structural::TopoSortError;
use crate::{Parallelism, centrality, pathfinding, structural};

const LABEL: &str = "T";
const LINK: &str = "link";

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

/// Build a directed graph from `edges` (pairs of creation-order indices) over
/// `node_count` `T`-labeled nodes wired with `link` edges. Returns the shared
/// graph plus the created NodeIds in creation order.
fn build_graph(node_count: usize, edges: &[(usize, usize)]) -> (SharedGraph, Vec<NodeId>) {
    let shared = SharedGraph::new(GraphId::new(9_100));
    let label = istr(LABEL);
    let link = istr(LINK);

    let mut nodes = Vec::with_capacity(node_count);
    {
        let mut txn = shared.begin_write();
        for _ in 0..node_count {
            nodes.push(
                txn.mutator()
                    .create_node(LabelSet::single(label), PropertyMap::new())
                    .unwrap(),
            );
        }
        for &(a, b) in edges {
            txn.mutator()
                .create_edge(link, nodes[a], nodes[b], PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();
    }
    (shared, nodes)
}

fn config() -> ProjectionConfig {
    ProjectionConfig {
        name: "p".to_string(),
        node_labels: vec![istr(LABEL)],
        edge_labels: vec![],
        weight_property: None,
    }
}

fn pagerank_config() -> PageRankConfig {
    PageRankConfig {
        damping: 0.85,
        max_iter: 100,
        tolerance: 1e-6,
        parallelism: Parallelism::Sequential,
    }
}

// ---- Projection lifecycle -------------------------------------------------

#[test]
fn projection_build_get_list_drop_roundtrip() {
    let (shared, _) = build_graph(3, &[(0, 1), (1, 2)]);
    let snapshot: Arc<SeleneGraph> = shared.read();
    let catalog = ProjectionCatalog::new();

    let (nodes, edges) = api::projection_build(&catalog, &snapshot, &config()).unwrap();
    assert_eq!(nodes, 3);
    assert_eq!(edges, 2);

    let info = api::projection_get(&catalog, &snapshot, "p").unwrap();
    assert_eq!(
        info,
        ProjectionInfo {
            name: "p".to_string(),
            generation: snapshot.meta.generation,
            node_count: 3,
            edge_count: 2,
        }
    );

    let listed = api::projection_list(&catalog, &snapshot).unwrap();
    assert_eq!(listed, vec![info]);

    assert!(api::projection_drop(&catalog, "p"));
    assert!(!api::projection_drop(&catalog, "p"));
    assert!(
        api::projection_list(&catalog, &snapshot)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn projection_get_missing_is_no_such_projection() {
    let (shared, _) = build_graph(1, &[]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();

    let err = api::projection_get(&catalog, &snapshot, "absent").unwrap_err();
    assert!(matches!(
        err,
        AlgorithmsError::NoSuchProjection { name } if name == "absent"
    ));
}

#[test]
fn runner_against_unbuilt_projection_is_projection_error() {
    let (shared, _) = build_graph(2, &[(0, 1)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();

    let err = api::pagerank(&catalog, &snapshot, "absent", pagerank_config()).unwrap_err();
    assert!(matches!(
        err,
        ApiError::Projection(AlgorithmsError::NoSuchProjection { .. })
    ));
}

// ---- Facade matches the underlying algorithm exactly ----------------------

#[test]
fn pagerank_facade_matches_direct_algorithm() {
    let (shared, _) = build_graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    api::projection_build(&catalog, &snapshot, &config()).unwrap();

    let via_facade = api::pagerank(&catalog, &snapshot, "p", pagerank_config()).unwrap();

    // Build the same projection directly and run the algorithm with no facade.
    let projection = crate::projection::GraphProjection::build(&snapshot, &config(), None).unwrap();
    let direct = centrality::pagerank(&projection, pagerank_config());

    assert_eq!(via_facade, direct);
}

#[test]
fn wcc_facade_matches_direct_algorithm() {
    // Two disjoint components: {0,1} and {2,3}.
    let (shared, _) = build_graph(4, &[(0, 1), (2, 3)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    api::projection_build(&catalog, &snapshot, &config()).unwrap();

    let via_facade = api::wcc(&catalog, &snapshot, "p").unwrap();
    let projection = crate::projection::GraphProjection::build(&snapshot, &config(), None).unwrap();
    let direct = structural::wcc(&projection);
    assert_eq!(via_facade, direct);
    assert_eq!(api::wcc_count(&catalog, &snapshot, "p").unwrap(), 2);
}

#[test]
fn dijkstra_facade_matches_direct_algorithm() {
    let (shared, ids) = build_graph(3, &[(0, 1), (1, 2)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    api::projection_build(&catalog, &snapshot, &config()).unwrap();

    let via_facade = api::dijkstra(&catalog, &snapshot, "p", ids[0], ids[2])
        .unwrap()
        .expect("path exists");

    let projection = crate::projection::GraphProjection::build(&snapshot, &config(), None).unwrap();
    let direct = pathfinding::dijkstra(&projection, ids[0], ids[2])
        .unwrap()
        .expect("path exists");
    assert_eq!(via_facade.cost, direct.cost);
    assert_eq!(via_facade.nodes, direct.nodes);
}

// ---- Extension trait parity ----------------------------------------------

#[test]
fn trait_methods_match_free_functions() {
    let (shared, ids) = build_graph(4, &[(0, 1), (1, 2), (2, 0), (2, 3)]);
    let snapshot: Arc<SeleneGraph> = shared.read();
    let graph: &SeleneGraph = &snapshot;
    let catalog = ProjectionCatalog::new();

    // Build via the trait, then assert every algorithm matches its free fn.
    let (n, e) = graph.projection_build(&catalog, &config()).unwrap();
    assert_eq!((n, e), (4, 4));

    assert_eq!(
        graph.pagerank(&catalog, "p", pagerank_config()).unwrap(),
        api::pagerank(&catalog, graph, "p", pagerank_config()).unwrap()
    );
    assert_eq!(
        graph
            .betweenness(
                &catalog,
                "p",
                BetweennessConfig {
                    sample_size: None,
                    parallelism: Parallelism::Sequential,
                }
            )
            .unwrap(),
        api::betweenness(
            &catalog,
            graph,
            "p",
            BetweennessConfig {
                sample_size: None,
                parallelism: Parallelism::Sequential,
            }
        )
        .unwrap()
    );
    assert_eq!(
        graph.wcc(&catalog, "p").unwrap(),
        api::wcc(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.scc(&catalog, "p").unwrap(),
        api::scc(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.wcc_count(&catalog, "p").unwrap(),
        api::wcc_count(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.scc_count(&catalog, "p").unwrap(),
        api::scc_count(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.label_propagation(&catalog, "p", 50).unwrap(),
        api::label_propagation(&catalog, graph, "p", 50).unwrap()
    );
    assert_eq!(
        graph.louvain(&catalog, "p", 50).unwrap(),
        api::louvain(&catalog, graph, "p", 50).unwrap()
    );
    assert_eq!(
        graph
            .triangle_count(
                &catalog,
                "p",
                TriangleCountConfig {
                    parallelism: Parallelism::Sequential,
                }
            )
            .unwrap(),
        api::triangle_count(
            &catalog,
            graph,
            "p",
            TriangleCountConfig {
                parallelism: Parallelism::Sequential,
            }
        )
        .unwrap()
    );
    assert_eq!(
        graph.articulation_points(&catalog, "p").unwrap(),
        api::articulation_points(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.bridges(&catalog, "p").unwrap(),
        api::bridges(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.sssp(&catalog, "p", ids[0]).unwrap(),
        api::sssp(&catalog, graph, "p", ids[0]).unwrap()
    );
    assert_eq!(
        graph.dijkstra(&catalog, "p", ids[0], ids[3]).unwrap(),
        api::dijkstra(&catalog, graph, "p", ids[0], ids[3]).unwrap()
    );
    assert_eq!(
        graph
            .apsp(
                &catalog,
                "p",
                ApspConfig {
                    max_nodes: 16,
                    parallelism: Parallelism::Sequential,
                }
            )
            .unwrap(),
        api::apsp(
            &catalog,
            graph,
            "p",
            ApspConfig {
                max_nodes: 16,
                parallelism: Parallelism::Sequential,
            }
        )
        .unwrap()
    );
    assert_eq!(
        graph.projection_get(&catalog, "p").unwrap(),
        api::projection_get(&catalog, graph, "p").unwrap()
    );
    assert_eq!(
        graph.projection_list(&catalog).unwrap(),
        api::projection_list(&catalog, graph).unwrap()
    );
}

// ---- Error-channel mapping ------------------------------------------------

#[test]
fn topological_sort_on_cycle_is_topo_sort_error() {
    // 0 -> 1 -> 0 is a directed cycle.
    let (shared, _) = build_graph(2, &[(0, 1), (1, 0)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    api::projection_build(&catalog, &snapshot, &config()).unwrap();

    let err = api::topological_sort(&catalog, &snapshot, "p").unwrap_err();
    assert!(matches!(
        err,
        ApiError::TopoSort(TopoSortError::NotADag { .. })
    ));
}

#[test]
fn apsp_over_node_limit_is_pathfinding_error() {
    let (shared, _) = build_graph(4, &[(0, 1), (1, 2), (2, 3)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    api::projection_build(&catalog, &snapshot, &config()).unwrap();

    let err = api::apsp(
        &catalog,
        &snapshot,
        "p",
        ApspConfig {
            max_nodes: 1,
            parallelism: Parallelism::Sequential,
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ApiError::Pathfinding(PathfindingError::TooLarge { nodes: 4, limit: 1 })
    ));
}

#[test]
fn cancelled_checker_aborts_runner() {
    let (shared, _) = build_graph(3, &[(0, 1), (1, 2)]);
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    api::projection_build(&catalog, &snapshot, &config()).unwrap();

    let token = CancellationToken::new();
    token.cancel();
    let checker = CancellationChecker::new(Some(&token), None);

    let err = api::pagerank_with_checker(&catalog, &snapshot, "p", pagerank_config(), checker)
        .unwrap_err();
    assert!(matches!(err, ApiError::Aborted(_)));
}

// ---- Generation staleness -------------------------------------------------

#[test]
fn runner_refreshes_stale_projection_after_graph_mutation() {
    let (shared, ids) = build_graph(3, &[(0, 1)]);
    let catalog = ProjectionCatalog::new();

    {
        let snapshot = shared.read();
        let (nodes, edges) = api::projection_build(&catalog, &snapshot, &config()).unwrap();
        assert_eq!((nodes, edges), (3, 1));
    }

    // Mutate the graph: add an edge so the projection becomes stale.
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_edge(istr(LINK), ids[1], ids[2], PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }

    // A fresh snapshot has a higher generation; the runner must rebuild the
    // projection (now reporting 2 edges) rather than serving the stale one.
    let snapshot = shared.read();
    let info = api::projection_get(&catalog, &snapshot, "p").unwrap();
    assert_eq!(info.edge_count, 2);
    assert_eq!(info.generation, snapshot.meta.generation);

    // wcc over the refreshed projection sees both edges (one component).
    assert_eq!(api::wcc_count(&catalog, &snapshot, "p").unwrap(), 1);
}
