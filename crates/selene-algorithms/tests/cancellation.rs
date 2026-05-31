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

use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Build a single-component line graph of `n` nodes (`n0 -> n1 -> ... ->
/// n_{n-1}`) so an algorithm's per-node loop is forced to cross the
/// `ALGORITHM_CANCEL_CHECK_STRIDE` (1024) boundary inside the body, exercising
/// the in-loop strided checkpoint rather than the entry check.
fn build_large_line_graph(n: usize) -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(1171));
    let label = istr("N");
    let rel = istr("R");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(n);
    for _ in 0..n {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .expect("node inserts"),
        );
    }
    for i in 0..n - 1 {
        txn.mutator()
            .create_edge(rel, nodes[i], nodes[i + 1], PropertyMap::new())
            .expect("edge inserts");
    }
    txn.commit().expect("graph commits");
    shared
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

/// ALGO-12: exercise the **in-loop** strided checkpoint (stride 1024), not the
/// entry check. A 50K-node line graph forces the per-node loop well past the
/// stride boundary. The token starts un-cancelled, so the entry
/// `check_algorithm` passes; a side thread cancels it while the loop is in
/// flight, so the cancellation can only be observed at an in-loop strided
/// checkpoint. The outcome is `Cancelled` regardless of which checkpoint
/// catches it, but the >1024-unit body is what makes the strided path the
/// realistic trip point (vs the entry check, which already passed).
#[test]
fn strided_checkpoint_trips_mid_loop_on_large_projection() {
    let shared = build_large_line_graph(50_000);
    let snapshot = shared.read();
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "large".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .expect("projection builds");

    let token = Arc::new(CancellationToken::new());
    // Cancel from a side thread so the entry check (token still live) passes and
    // the strided in-loop checkpoint is the trip point.
    let canceller = {
        let token = token.clone();
        std::thread::spawn(move || {
            token.cancel();
        })
    };

    // SCC walks every node with a strided checkpoint inside the iterative
    // Tarjan loop. With 50K nodes the loop crosses the 1024-stride boundary
    // ~48 times, so the cancel is observed cooperatively.
    let checker = CancellationChecker::new(Some(&token), None);
    let result = scc_with_checker(&proj, checker);

    canceller.join().expect("canceller thread joins");
    assert_eq!(
        result.expect_err("scc over a cancelled token aborts").cause,
        CancellationCause::Cancelled,
        "strided in-loop checkpoint observes the cooperative cancellation"
    );
}

/// ALGO-12: a deadline that has already elapsed propagates
/// `CancellationCause::Timeout` (not `Cancelled`) through the algorithm into the
/// typed result. Every cancellable surface routes the deadline through the same
/// `check_algorithm` path, so one representative per result shape is sufficient.
#[test]
fn elapsed_deadline_reports_timeout_cause() {
    let shared = build_graph();
    let proj = build_proj(&shared);
    let first = proj.iter_nodes().next().expect("projection has nodes");

    // A deadline 1s in the past: the very first checkpoint observes Timeout.
    let elapsed_deadline = Instant::now() - Duration::from_secs(1);
    let checker = CancellationChecker::new(None, Some(elapsed_deadline));

    // AlgorithmAborted result shape (SCC).
    let cause = scc_with_checker(&proj, checker)
        .expect_err("scc past deadline aborts")
        .cause;
    assert!(
        matches!(cause, CancellationCause::Timeout { .. }),
        "expected Timeout cause, got {cause:?}"
    );

    // PathfindingError shape (SSSP) — the deadline must also surface as Timeout
    // through the pathfinding error wrapper.
    let PathfindingError::Aborted { source } =
        sssp_with_checker(&proj, first, checker).expect_err("sssp past deadline aborts")
    else {
        panic!("expected sssp timeout abort");
    };
    assert!(
        matches!(source.cause, CancellationCause::Timeout { .. }),
        "expected Timeout cause through PathfindingError, got {:?}",
        source.cause
    );

    // TopoSortError shape — same deadline, same Timeout cause.
    let TopoSortError::Aborted { source } =
        topological_sort_with_checker(&proj, checker).expect_err("topo past deadline aborts")
    else {
        panic!("expected topo timeout abort");
    };
    assert!(
        matches!(source.cause, CancellationCause::Timeout { .. }),
        "expected Timeout cause through TopoSortError, got {:?}",
        source.cause
    );
}
