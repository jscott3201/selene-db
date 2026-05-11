//! Deterministic rendering for algorithm snapshot fixtures (M5f closure;
//! D21 pattern; spec 16 §E31–§E36).
//!
//! Gated `#[cfg(any(test, feature = "test-harness"))]` so production builds
//! ship zero rendering overhead (§E34). The renderer takes
//! algorithm-emitted result vectors and produces a stable textual form for
//! `insta::assert_snapshot!` comparison.
//!
//! Snapshot-determinism (§E31): `NodeId` rendered as `n<sparse_row>` (row =
//! `NodeId.get() - 1` per the §E20 invariant in selene-graph's store, valid
//! when no tombstones intervene — corpus fixtures are append-only); `f64`
//! scores rendered as `{:.6}`; result vectors NOT re-sorted at render time
//! (preserves §E12/§E17/§E21/§E27 algorithm-emitted order); empty results
//! rendered as the literal token `RESULT EMPTY`.

use std::fmt;

use selene_core::NodeId;

use crate::pathfinding::PathResult;

/// Input accepted by [`algo_summary`].
#[derive(Debug)]
pub struct AlgoSnapshotInput<'a> {
    /// Summary of the projection the algorithm ran against.
    pub graph_summary: GraphSummary<'a>,
    /// Stable algorithm name (e.g., `"pagerank"`, `"triangle_count"`).
    pub algorithm: &'a str,
    /// Stable parameter summary (e.g., `"damping=0.85 max_iter=100 tolerance=0.000001"`).
    pub params: &'a str,
    /// Result to render.
    pub result: AlgoResult<'a>,
}

/// Stable projection summary surfaced in the snapshot header.
#[derive(Debug)]
pub struct GraphSummary<'a> {
    /// Fixture slug (matches the corpus entry's `AlgoCorpusGraph` debug name,
    /// e.g., `"k3-cycle"` or `"sparse-row-triangle{total=100,scope=3}"`).
    pub fixture_label: &'a str,
    /// Number of nodes in the projection.
    pub node_count: usize,
    /// Number of outgoing edges in the projection (directed count).
    pub edge_count: usize,
}

/// Closed enumeration of algorithm result shapes the harness renders.
#[derive(Debug)]
#[non_exhaustive]
pub enum AlgoResult<'a> {
    /// `Vec<(NodeId, u64)>` — used by `wcc`, `scc`, `label_propagation`.
    NodePairs(&'a [(NodeId, u64)]),
    /// `Vec<(NodeId, u64, u32)>` — used by `louvain`.
    NodePairsWithLevel(&'a [(NodeId, u64, u32)]),
    /// `Vec<(NodeId, f64)>` — used by `pagerank`, `betweenness`, `sssp`.
    NodePairsFloat(&'a [(NodeId, f64)]),
    /// `Vec<(NodeId, usize)>` — used by `triangle_count`.
    NodePairsUsize(&'a [(NodeId, usize)]),
    /// `Vec<(NodeId, usize)>` where usize is the topo position — used by
    /// `topological_sort`.
    TopologicalOrder(&'a [(NodeId, usize)]),
    /// `Vec<NodeId>` — used by `articulation_points`.
    NodeList(&'a [NodeId]),
    /// `Vec<(NodeId, NodeId)>` — used by `bridges`.
    EdgeList(&'a [(NodeId, NodeId)]),
    /// `usize` — used by `wcc_count`, `scc_count`.
    Count(usize),
    /// `Option<PathResult>` — used by `dijkstra`.
    DijkstraPath(Option<&'a PathResult>),
    /// `Vec<(NodeId, NodeId, f64)>` — used by `apsp`.
    ApspMatrix(&'a [(NodeId, NodeId, f64)]),
    /// Stable short error label — used when a fallible algorithm errors.
    /// E.g., `"NotADag"`, `"NegativeWeight"`, `"TooLarge"`.
    Err(&'a str),
}

/// Rendered snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlgoSnapshot {
    /// Section lines in render order. `Display` joins with `'\n'`.
    pub lines: Vec<String>,
}

impl fmt::Display for AlgoSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            f.write_str(line)?;
        }
        Ok(())
    }
}

/// Render an algorithm input + result into a stable textual snapshot.
#[must_use]
pub fn algo_summary(input: &AlgoSnapshotInput<'_>) -> AlgoSnapshot {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "GRAPH {{ fixture: {:?}, nodes: {}, edges: {} }}",
        input.graph_summary.fixture_label,
        input.graph_summary.node_count,
        input.graph_summary.edge_count
    ));
    lines.push(format!(
        "INVOCATION {{ algorithm: {:?}, params: {:?} }}",
        input.algorithm, input.params
    ));
    render_result(&input.result, &mut lines);
    AlgoSnapshot { lines }
}

fn render_result(result: &AlgoResult<'_>, out: &mut Vec<String>) {
    match result {
        AlgoResult::NodePairs(pairs) => {
            if pairs.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (nid, label) in *pairs {
                out.push(format!("  {} -> label={}", render_node(*nid), label));
            }
        }
        AlgoResult::NodePairsWithLevel(triples) => {
            if triples.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (nid, comm, level) in *triples {
                out.push(format!(
                    "  {} -> community={} level={}",
                    render_node(*nid),
                    comm,
                    level
                ));
            }
        }
        AlgoResult::NodePairsFloat(pairs) => {
            if pairs.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (nid, score) in *pairs {
                out.push(format!("  {} -> {:.6}", render_node(*nid), score));
            }
        }
        AlgoResult::NodePairsUsize(pairs) => {
            if pairs.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (nid, count) in *pairs {
                out.push(format!("  {} -> count={}", render_node(*nid), count));
            }
        }
        AlgoResult::TopologicalOrder(pairs) => {
            if pairs.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (nid, pos) in *pairs {
                out.push(format!("  {} -> position={}", render_node(*nid), pos));
            }
        }
        AlgoResult::NodeList(nodes) => {
            if nodes.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for nid in *nodes {
                out.push(format!("  {}", render_node(*nid)));
            }
        }
        AlgoResult::EdgeList(edges) => {
            if edges.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (src, dst) in *edges {
                out.push(format!("  {} -- {}", render_node(*src), render_node(*dst)));
            }
        }
        AlgoResult::Count(n) => {
            out.push(format!("RESULT COUNT {}", n));
        }
        AlgoResult::DijkstraPath(None) => {
            out.push("RESULT NO_PATH".to_string());
        }
        AlgoResult::DijkstraPath(Some(path)) => {
            out.push(format!("RESULT PATH cost={:.6}", path.cost));
            let rendered: Vec<String> = path.nodes.iter().map(|n| render_node(*n)).collect();
            out.push(format!("  nodes: [{}]", rendered.join(", ")));
        }
        AlgoResult::ApspMatrix(triples) => {
            if triples.is_empty() {
                out.push("RESULT EMPTY".to_string());
                return;
            }
            out.push("RESULT".to_string());
            for (src, dst, cost) in *triples {
                out.push(format!(
                    "  {} -> {} cost={:.6}",
                    render_node(*src),
                    render_node(*dst),
                    cost
                ));
            }
        }
        AlgoResult::Err(label) => {
            out.push(format!("RESULT ERR {}", label));
        }
    }
}

/// Render a `NodeId` as `n<sparse_row>` per §E31. Requires `NodeId.get() >= 1`
/// (the §E20 invariant in selene-graph's store: row 0 = NodeId 1; row N-1 =
/// NodeId N). Corpus fixtures are append-only so the row mapping is always
/// `row = NodeId.get() - 1`.
///
/// Rejects `NodeId(0)` — the tombstone sentinel per D11 — with a distinct
/// `<tombstone>` token rather than saturating to `n0`. This protects the
/// snapshot harness's role as a drift catcher: if an algorithm regresses
/// and surfaces a tombstone, the alias must NOT collide with a valid row 0
/// node. (PR #62 Codex P2 finding.)
fn render_node(nid: NodeId) -> String {
    let raw = nid.get();
    if raw == 0 {
        return "<tombstone>".to_string();
    }
    let row = raw - 1;
    format!("n{}", row)
}
