//! Cooperative cancellation coverage for cancellable algorithm variants.

use selene_algorithms::{
    ApspConfig, BetweennessConfig, GraphProjection, PageRankConfig, Parallelism, PathfindingError,
    ProjectionConfig, TopoSortError, TriangleCountConfig, apsp_with_checker,
    articulation_points_with_checker, betweenness_with_checker, bridges_with_checker,
    dijkstra_with_checker, label_propagation_with_checker, louvain_with_checker,
    pagerank_with_checker, scc_count_with_checker, scc_with_checker, sssp_with_checker,
    topological_sort_with_checker, triangle_count_with_checker, wcc_count_with_checker,
    wcc_with_checker,
};
use selene_core::{
    CancellationCause, CancellationChecker, CancellationToken, GraphId, IStr, LabelSet,
    PropertyMap, intern,
};
use selene_graph::SharedGraph;

fn istr(name: &str) -> IStr {
    intern(name).expect("test name interns")
}

fn build_proj(shared: &SharedGraph) -> GraphProjection {
    let snapshot = shared.read();
    GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "test".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .expect("projection builds")
}

fn build_graph() -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(1170));
    let label = istr("N");
    let rel = istr("R");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::new();
    for _ in 0..4 {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .expect("node inserts"),
        );
    }
    for &(s, t) in &[(0, 1), (1, 2), (2, 0), (2, 3)] {
        txn.mutator()
            .create_edge(rel, nodes[s], nodes[t], PropertyMap::new())
            .expect("edge inserts");
    }
    txn.commit().expect("graph commits");
    shared
}

fn cancelled_checker<'a>(token: &'a CancellationToken) -> CancellationChecker<'a> {
    token.cancel();
    CancellationChecker::new(Some(token), None)
}

#[test]
fn cancellable_algorithm_variants_report_cancelled() {
    let shared = build_graph();
    let proj = build_proj(&shared);
    let first = proj.iter_nodes().next().expect("projection has nodes");
    let second = proj
        .iter_nodes()
        .nth(1)
        .expect("projection has second node");
    let token = CancellationToken::new();
    let checker = cancelled_checker(&token);

    assert_eq!(
        pagerank_with_checker(
            &proj,
            PageRankConfig {
                damping: 0.85,
                max_iter: 10,
                tolerance: 1e-6,
                parallelism: Parallelism::Sequential,
            },
            checker,
        )
        .expect_err("pagerank aborts")
        .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        betweenness_with_checker(
            &proj,
            BetweennessConfig {
                sample_size: None,
                parallelism: Parallelism::Sequential,
            },
            checker,
        )
        .expect_err("betweenness aborts")
        .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        label_propagation_with_checker(&proj, 10, checker)
            .expect_err("label propagation aborts")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        louvain_with_checker(&proj, 10, checker)
            .expect_err("louvain aborts")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        triangle_count_with_checker(
            &proj,
            TriangleCountConfig {
                parallelism: Parallelism::Sequential,
            },
            checker,
        )
        .expect_err("triangle count aborts")
        .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        wcc_with_checker(&proj, checker)
            .expect_err("wcc aborts")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        wcc_count_with_checker(&proj, checker)
            .expect_err("wcc count aborts")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        scc_with_checker(&proj, checker)
            .expect_err("scc aborts")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        scc_count_with_checker(&proj, checker)
            .expect_err("scc count aborts")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        articulation_points_with_checker(&proj, checker)
            .expect_err("articulation points abort")
            .cause,
        CancellationCause::Cancelled
    );
    assert_eq!(
        bridges_with_checker(&proj, checker)
            .expect_err("bridges abort")
            .cause,
        CancellationCause::Cancelled
    );

    let PathfindingError::Aborted { source } =
        dijkstra_with_checker(&proj, first, second, checker).expect_err("dijkstra aborts")
    else {
        panic!("expected dijkstra cancellation");
    };
    assert_eq!(source.cause, CancellationCause::Cancelled);

    let PathfindingError::Aborted { source } =
        sssp_with_checker(&proj, first, checker).expect_err("sssp aborts")
    else {
        panic!("expected sssp cancellation");
    };
    assert_eq!(source.cause, CancellationCause::Cancelled);

    let PathfindingError::Aborted { source } = apsp_with_checker(
        &proj,
        ApspConfig {
            max_nodes: 10,
            parallelism: Parallelism::Sequential,
        },
        checker,
    )
    .expect_err("apsp aborts") else {
        panic!("expected apsp cancellation");
    };
    assert_eq!(source.cause, CancellationCause::Cancelled);

    let TopoSortError::Aborted { source } =
        topological_sort_with_checker(&proj, checker).expect_err("topological sort aborts")
    else {
        panic!("expected topological-sort cancellation");
    };
    assert_eq!(source.cause, CancellationCause::Cancelled);
}
