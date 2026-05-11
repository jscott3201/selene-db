//! Filtered subgraph view with cached CSR adjacency for algorithm computation.
//!
//! A [`GraphProjection`] is a frozen view of the graph at a given generation,
//! defined by:
//! - A node bitmap (row-indexed, AND-intersected with optional scope).
//! - An edge-label filter (which edge types appear in the CSR).
//! - An optional weight property (numeric values project to `f64`; missing /
//!   non-numeric / null values default to `1.0` per spec 16 §E04).
//! - Cached out-direction and in-direction CSR adjacency.
//!
//! Projections are immutable once built. When the underlying graph mutates and
//! its `meta.generation` advances, the projection is logically stale —
//! BRIEF-51 lands a `ProjectionCatalog` that rebuilds projections from stored
//! configs when staleness is detected.

mod csr;

use roaring::RoaringBitmap;
use selene_core::{IStr, NodeId};
use selene_graph::{SeleneGraph, store::node_row_index};

pub use csr::ProjNeighbor;
use csr::{ProjCsr, build_csr_in, build_csr_out};

use crate::error::AlgorithmsError;

/// Configuration for creating a graph projection.
///
/// `ProjectionConfig` is the user-facing input surface; literal construction
/// via struct expression is part of the ergonomic contract. Fields added later
/// land via a future builder pattern rather than via `#[non_exhaustive]`.
#[derive(Debug, Clone)]
pub struct ProjectionConfig {
    /// Stable name used by the projection catalog (BRIEF-51). Projection names
    /// are user-facing and arbitrary; `String` keeps them out of the global
    /// `IStr` interner per the spec 16 §E04 high-cardinality discipline.
    pub name: String,
    /// Node labels to include. Empty = all alive nodes (intersected with
    /// `scope` at build time).
    pub node_labels: Vec<IStr>,
    /// Edge labels to include. Empty = all edge types.
    pub edge_labels: Vec<IStr>,
    /// Property key projecting numeric edge weights to `f64`. `None` =
    /// unweighted (all weights = `1.0`).
    ///
    /// Edges lacking this property, or carrying a non-numeric value
    /// (including `Value::Null`), default to weight `1.0` (the same as the
    /// unweighted case). For strict weight validation, preprocess at write
    /// time.
    pub weight_property: Option<IStr>,
}

/// A named subgraph view with cached CSR adjacency for fast algorithm
/// traversal.
///
/// The projection is immutable once created. When the underlying graph mutates
/// (generation changes), the projection is logically stale; BRIEF-51's catalog
/// invalidates and rebuilds on staleness detection.
#[derive(Debug)]
pub struct GraphProjection {
    name: String,
    /// Row-indexed bitmap of nodes included in this projection (post label
    /// filter, post scope intersection).
    nodes: RoaringBitmap,
    edge_labels: Vec<IStr>,
    weight_property: Option<IStr>,
    out_csr: ProjCsr,
    in_csr: ProjCsr,
    generation: u64,
}

impl GraphProjection {
    /// Build a projection from a frozen graph snapshot.
    ///
    /// Returns `Ok(projection)` even when the filtered node set is empty — an
    /// empty subgraph is a legitimate algorithm input (e.g., "PageRank over a
    /// graph with zero `Person` nodes returns empty"). Algorithms requiring
    /// non-empty input check `node_count() > 0` after build and may raise
    /// [`AlgorithmsError::EmptyProjection`] from their own surface.
    pub fn build(
        snapshot: &SeleneGraph,
        config: &ProjectionConfig,
        scope: Option<&RoaringBitmap>,
    ) -> Result<Self, AlgorithmsError> {
        // Step 1: compute the row-indexed node bitmap.
        let mut nodes = if config.node_labels.is_empty() {
            snapshot.live_nodes().clone()
        } else {
            let mut bm = RoaringBitmap::new();
            for label in &config.node_labels {
                if let Some(label_bm) = snapshot.nodes_with_label(label) {
                    bm |= label_bm;
                }
            }
            // Restrict to alive rows in case any label bitmap retains a stale
            // entry (defensive — the mutation funnel keeps these in sync).
            bm &= snapshot.live_nodes();
            bm
        };
        if let Some(scope_bm) = scope {
            nodes &= scope_bm;
        }

        // Step 2: build CSR adjacency for each direction.
        let out_csr = build_csr_out(
            snapshot,
            &nodes,
            &config.edge_labels,
            config.weight_property.as_ref(),
        );
        let in_csr = build_csr_in(
            snapshot,
            &nodes,
            &config.edge_labels,
            config.weight_property.as_ref(),
        );

        Ok(Self {
            name: config.name.clone(),
            nodes,
            edge_labels: config.edge_labels.clone(),
            weight_property: config.weight_property,
            out_csr,
            in_csr,
            generation: snapshot.meta.generation,
        })
    }

    /// Projection name (from `ProjectionConfig::name`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Number of nodes in the projection.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len() as usize
    }

    /// Number of **outgoing** edges in this projection.
    ///
    /// For directed graphs this is the total edge count; for algorithms
    /// treating the projection as undirected (e.g., WCC, label propagation),
    /// iterate `out_neighbors()` ∪ `in_neighbors()` per node to avoid
    /// double-counting — do NOT sum `out_degree() + in_degree()` over all
    /// nodes.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.out_csr.total_neighbors()
    }

    /// Graph generation pinned at build time. BRIEF-51's catalog compares this
    /// against `snapshot.meta.generation` for staleness detection.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns true when `node` is part of this projection.
    #[must_use]
    pub fn contains(&self, node: NodeId) -> bool {
        match node_row_index(node) {
            Some(row) => self.nodes.contains(row),
            None => false,
        }
    }

    /// Out-neighbors of `node`, sorted ASC by `node_id` per spec 16 §E03.
    ///
    /// Returns an empty slice when `node` is not in this projection or has no
    /// qualifying outgoing edges.
    #[must_use]
    pub fn out_neighbors(&self, node: NodeId) -> &[ProjNeighbor] {
        let Some(row) = node_row_index(node) else {
            return &[];
        };
        if !self.nodes.contains(row) {
            return &[];
        }
        self.out_csr.neighbors_of_row(row)
    }

    /// In-neighbors of `node`, sorted ASC by `node_id` per spec 16 §E03.
    ///
    /// Returns an empty slice when `node` is not in this projection or has no
    /// qualifying incoming edges.
    #[must_use]
    pub fn in_neighbors(&self, node: NodeId) -> &[ProjNeighbor] {
        let Some(row) = node_row_index(node) else {
            return &[];
        };
        if !self.nodes.contains(row) {
            return &[];
        }
        self.in_csr.neighbors_of_row(row)
    }

    /// Out-degree of `node` within this projection.
    #[must_use]
    pub fn out_degree(&self, node: NodeId) -> usize {
        self.out_neighbors(node).len()
    }

    /// In-degree of `node` within this projection.
    #[must_use]
    pub fn in_degree(&self, node: NodeId) -> usize {
        self.in_neighbors(node).len()
    }

    /// Iterate node IDs in the projection in **ascending order** (inherited
    /// from `RoaringBitmap` iteration).
    ///
    /// Use for deterministic traversal in tests and snapshot goldens; algorithm
    /// correctness on ties (BFS visit order, SCC enumeration order, Louvain
    /// community-id assignment on equal modularity) depends on this stability.
    pub fn iter_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().map(|row| NodeId::new(u64::from(row) + 1))
    }

    /// Returns true when this projection carries weighted edges.
    #[must_use]
    pub fn is_weighted(&self) -> bool {
        self.weight_property.is_some()
    }

    /// Edge-label filter declared at projection build time. Empty slice means
    /// "all edge types were admitted" (no filter).
    #[must_use]
    pub fn edge_labels(&self) -> &[IStr] {
        &self.edge_labels
    }

    /// Maximum row index in the projection's node bitmap, if any.
    ///
    /// Used by intra-crate algorithm modules to size state arrays
    /// (e.g., Tarjan's `disc` / `low` arrays sized to `max_row + 1` per
    /// donor `aether-db-algorithms/src/structural.rs:162`).
    #[must_use]
    pub(crate) fn max_row(&self) -> Option<u32> {
        self.nodes.max()
    }
}
