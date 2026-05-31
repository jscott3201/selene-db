//! Deep-chain stack-safety regression for the iterative work-stack DFS
//! algorithms (Tarjan SCC, articulation points, bridges) — ALGO-04.
//!
//! The entire iterative-work-stack design (`structural/components.rs`,
//! `structural/articulation.rs`) exists so these algorithms survive
//! `10^7`-node graphs without blowing the call stack. Every other structural
//! fixture is a handful of nodes, so a recursive regression (e.g. a refactor
//! reintroducing `fn strongconnect(...) { ...; strongconnect(child); }`) would
//! overflow only in production and pass CI silently.
//!
//! This test builds a deep line graph and runs each DFS algorithm inside a
//! thread with a deliberately small stack (512 KiB). The iterative
//! implementations use heap-allocated work stacks, so they fit comfortably; a
//! recursive implementation at this depth would overflow the small stack and
//! abort the process, failing the test. The results are also checked exactly so
//! the test guards correctness, not just survival.

use std::thread;

use selene_algorithms::{
    GraphProjection, ProjectionConfig, articulation_points, bridges, scc, scc_count,
};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::SharedGraph;

/// Line-graph depth. Large enough that a recursive DFS would overflow the
/// 512 KiB worker stack (each recursive frame carries the projection borrow,
/// neighbor cursor, and lowlink bookkeeping — tens of bytes minimum, so
/// `DEPTH` frames blow well past 512 KiB), yet small enough to build + run in
/// well under a second.
const DEPTH: usize = 100_000;

/// Small worker-thread stack. A correct iterative impl keeps its work stack on
/// the heap and needs only O(1) native stack; a recursive impl would need
/// `O(DEPTH)` native frames and overflow this.
const WORKER_STACK_BYTES: usize = 512 * 1024;

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

/// Build a directed line graph `n0 -> n1 -> ... -> n_{depth-1}` in a single
/// transaction. Returns the shared graph and the ordered NodeIds.
fn build_line(depth: usize) -> (SharedGraph, Vec<NodeId>) {
    let shared = SharedGraph::new(GraphId::new(4_004));
    let label = istr("N");
    let rel = istr("R");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(depth);
    for _ in 0..depth {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .unwrap(),
        );
    }
    for i in 0..depth - 1 {
        txn.mutator()
            .create_edge(rel, nodes[i], nodes[i + 1], PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    (shared, nodes)
}

fn build_proj(shared: &SharedGraph) -> GraphProjection {
    let snapshot = shared.read();
    GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "deep".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap()
}

/// Run `body` on a thread with a small stack and propagate panics/overflows as
/// a test failure.
fn run_on_small_stack<F>(name: &str, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let handle = thread::Builder::new()
        .name(name.to_string())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(body)
        .expect("worker thread spawns");
    handle
        .join()
        .unwrap_or_else(|_| panic!("{name} overflowed the small worker stack (recursive DFS?)"));
}

#[test]
fn scc_deep_line_graph_no_stack_overflow() {
    let (shared, nodes) = build_line(DEPTH);
    run_on_small_stack("scc-deep", move || {
        let proj = build_proj(&shared);

        // A directed line graph is acyclic: every node is its own SCC.
        assert_eq!(scc_count(&proj), DEPTH);

        let result = scc(&proj);
        assert_eq!(result.len(), DEPTH);
        // Each node maps to itself as the component id (singleton SCC), and the
        // result is sorted ASC by NodeId.
        for (i, &(nid, comp)) in result.iter().enumerate() {
            assert_eq!(nid, nodes[i], "scc result must be ASC by NodeId");
            assert_eq!(comp, nodes[i].get(), "each node is its own singleton SCC");
        }
    });
}

#[test]
fn articulation_deep_line_graph_no_stack_overflow() {
    let (shared, nodes) = build_line(DEPTH);
    run_on_small_stack("articulation-deep", move || {
        let proj = build_proj(&shared);

        // In a path's undirected view, every interior node (all but the two
        // endpoints) is an articulation point.
        let ap = articulation_points(&proj);
        assert_eq!(ap.len(), DEPTH - 2);
        // Sorted ASC, and exactly nodes[1..DEPTH-1].
        for (offset, &node) in ap.iter().enumerate() {
            assert_eq!(node, nodes[offset + 1]);
        }
    });
}

#[test]
fn bridges_deep_line_graph_no_stack_overflow() {
    let (shared, nodes) = build_line(DEPTH);
    run_on_small_stack("bridges-deep", move || {
        let proj = build_proj(&shared);

        // Every edge of a path is a bridge.
        let result = bridges(&proj);
        assert_eq!(result.len(), DEPTH - 1);
        // Bridges sorted ASC by (source, target); each is the canonicalized
        // consecutive pair (nodes[i], nodes[i+1]) with source < target.
        for (i, &(a, b)) in result.iter().enumerate() {
            let lo = nodes[i].get().min(nodes[i + 1].get());
            let hi = nodes[i].get().max(nodes[i + 1].get());
            assert_eq!(a.get(), lo);
            assert_eq!(b.get(), hi);
        }
    });
}
