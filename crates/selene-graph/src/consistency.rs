//! Debug-only structural consistency net for built-in graph indexes.
//!
//! Every selene-graph derived index — the label / edge-label bitmaps, the
//! per-`(label, property)` typed indexes, the composite typed indexes, and the
//! in/out adjacency maps — is maintained incrementally on the commit path
//! (`crate::mutator`) and rebuilt wholesale on the snapshot-load /
//! recovery path (`crate::shared::rebuild_derived_state` +
//! `crate::property_index::rebuild_property_indexes` +
//! `crate::composite_property_index::rebuild_composite_property_indexes` +
//! `crate::vector_index::rebuild_vector_indexes`). A
//! bug in either path corrupts query results silently — the engine keeps
//! answering, just with wrong rows.
//!
//! [`SeleneGraph::assert_indexes_consistent`] re-derives every index from the
//! authoritative columns (`node_store`, `edge_store`) and compares it against
//! the maintained state, returning a descriptive [`Err`] on the first
//! mismatch. It is wired behind `#[cfg(debug_assertions)]` at the two graph
//! publication seams (the write-transaction commit and the snapshot-load
//! constructor) so every debug/test build self-checks each published snapshot
//! at zero release cost.

use roaring::RoaringBitmap;

use selene_core::{EdgeId, IStr, NodeId};

use crate::adjacency::AdjacencyEdge;
use crate::graph::SeleneGraph;

impl SeleneGraph {
    /// Re-derive every built-in index from the authoritative node/edge columns
    /// and assert it matches the maintained index state.
    ///
    /// Returns `Ok(())` when the snapshot is internally consistent, or
    /// `Err(message)` describing the first detected drift. Intended for
    /// debug assertions and tests — it allocates fresh reference indexes and
    /// is `O(nodes * indexes + edges)` per call.
    ///
    /// # Checked invariant families
    ///
    /// 1. **Label / edge-label bitmaps** match a re-derivation from alive node
    ///    label sets and alive edge labels; no bucket is present-but-empty.
    /// 2. **Typed property indexes** match a fresh lenient re-build
    ///    (`build_property_index_lenient`). The lenient policy is reused so
    ///    open-graph kind drift and NaN — which the commit path legitimately
    ///    skips — do not false-positive.
    /// 3. **Composite typed indexes** match a fresh lenient re-build, same
    ///    skip-aware policy.
    /// 4. **Vector row-set indexes** match a fresh lenient re-build, same
    ///    skip-aware policy.
    /// 5. **Store integrity / alive-set parity**: per-store columns share one
    ///    length and every alive row index is in range. Dead rows are
    ///    permitted holes (D11) and are only asserted absent from derived
    ///    state, never from the columns. The snapshot's `meta.next_*_id`
    ///    allocator high-water marks are intentionally **not** checked here:
    ///    the published snapshot's `meta` is allowed to carry stale allocator
    ///    fields after a `from_graph` / recovery load (the real allocator
    ///    floor is enforced separately by `IdAllocator::from_meta_with_floors`),
    ///    so they are not a derived-index invariant.
    /// 6. **Adjacency** matches a re-derivation from alive edges in both
    ///    directions, with no present-but-empty entry.
    ///
    /// # Errors
    ///
    /// Returns the first mismatch as a human-readable `String`.
    pub fn assert_indexes_consistent(&self) -> Result<(), String> {
        self.check_store_integrity()?;
        self.check_label_index()?;
        self.check_edge_label_index()?;
        self.check_property_indexes()?;
        self.check_composite_property_indexes()?;
        self.check_vector_indexes()?;
        self.check_adjacency()?;
        Ok(())
    }

    /// Family (4): column lengths agree, alive rows are in range, allocator
    /// floors clear the highest alive row.
    fn check_store_integrity(&self) -> Result<(), String> {
        let node_len = self.node_store.labels.len();
        if self.node_store.properties.len() != node_len {
            return Err(format!(
                "node store column length mismatch: labels={node_len} properties={}",
                self.node_store.properties.len()
            ));
        }
        let edge_len = self.edge_store.label.len();
        if self.edge_store.source.len() != edge_len
            || self.edge_store.target.len() != edge_len
            || self.edge_store.properties.len() != edge_len
        {
            return Err(format!(
                "edge store column length mismatch: label={edge_len} source={} target={} \
                 properties={}",
                self.edge_store.source.len(),
                self.edge_store.target.len(),
                self.edge_store.properties.len()
            ));
        }

        for row in self.node_store.alive.iter() {
            if (row as usize) >= node_len {
                return Err(format!(
                    "node alive bitmap references out-of-range row {row} (node store len {node_len})"
                ));
            }
        }
        for row in self.edge_store.alive.iter() {
            if (row as usize) >= edge_len {
                return Err(format!(
                    "edge alive bitmap references out-of-range row {row} (edge store len {edge_len})"
                ));
            }
        }
        Ok(())
    }

    /// Family (1a): node label bitmaps.
    fn check_label_index(&self) -> Result<(), String> {
        let mut reference: imbl::HashMap<IStr, RoaringBitmap> = imbl::HashMap::new();
        for row in self.node_store.alive.iter() {
            let Some(labels) = self.node_store.labels.get(row as usize) else {
                return Err(format!("alive node row {row} has no label column entry"));
            };
            for label in labels.iter() {
                reference.entry(label.clone()).or_default().insert(row);
            }
        }
        compare_bitmap_index("node label index", &self.idx_label, &reference)
    }

    /// Family (1b): edge label bitmaps.
    fn check_edge_label_index(&self) -> Result<(), String> {
        let mut reference: imbl::HashMap<IStr, RoaringBitmap> = imbl::HashMap::new();
        for row in self.edge_store.alive.iter() {
            let Some(label) = self.edge_store.label.get(row as usize) else {
                return Err(format!("alive edge row {row} has no label column entry"));
            };
            reference.entry(label.clone()).or_default().insert(row);
        }
        compare_bitmap_index("edge label index", &self.idx_edge_label, &reference)
    }

    /// Family (2): per-`(label, property)` typed indexes.
    fn check_property_indexes(&self) -> Result<(), String> {
        for ((label, property), entry) in &self.property_index {
            if entry.index.has_empty_bucket() {
                return Err(format!(
                    "property index ({label}, {property}) holds a present-but-empty bucket"
                ));
            }
            let reference = crate::property_index::build_property_index_lenient(
                self,
                label.clone(),
                property.clone(),
                entry.kind(),
            )
            .map_err(|err| {
                format!("failed to re-derive property index ({label}, {property}): {err}")
            })?;
            if !entry.index.buckets_eq(&reference) {
                return Err(format!(
                    "property index ({label}, {property}) drifted from a fresh re-derivation \
                     (maintained cardinality {}, reference cardinality {})",
                    entry.index.cardinality(),
                    reference.cardinality(),
                ));
            }
        }
        Ok(())
    }

    /// Family (3): composite typed indexes.
    fn check_composite_property_indexes(&self) -> Result<(), String> {
        for ((label, _key), entry) in &self.composite_property_index {
            if entry.index.has_empty_bucket() {
                return Err(format!(
                    "composite index ({label}, {:?}) holds a present-but-empty bucket",
                    entry.declared_properties
                ));
            }
            let reference =
                crate::composite_property_index::build_composite_property_index_lenient(
                    self,
                    label.clone(),
                    entry.declared_properties.clone(),
                    entry.kinds(),
                )
                .map_err(|err| {
                    format!(
                        "failed to re-derive composite index ({label}, {:?}): {err}",
                        entry.declared_properties
                    )
                })?;
            if !entry.index.buckets_eq(&reference) {
                return Err(format!(
                    "composite index ({label}, {:?}) drifted from a fresh re-derivation \
                     (maintained cardinality {}, reference cardinality {})",
                    entry.declared_properties,
                    entry.index.cardinality(),
                    reference.cardinality(),
                ));
            }
        }
        Ok(())
    }

    /// Family (4): vector row-set indexes.
    fn check_vector_indexes(&self) -> Result<(), String> {
        for ((label, property), entry) in &self.vector_index {
            let reference = crate::vector_index::build_vector_index_lenient(
                self,
                label.clone(),
                property.clone(),
                entry.kind(),
                entry.dimension(),
            )
            .map_err(|err| {
                format!("failed to re-derive vector index ({label}, {property}): {err}")
            })?;
            if !entry.index.rows_eq(&reference) {
                return Err(format!(
                    "vector index ({label}, {property}) drifted from a fresh re-derivation \
                     (maintained cardinality {}, reference cardinality {})",
                    entry.index.cardinality(),
                    reference.cardinality(),
                ));
            }
        }
        Ok(())
    }

    /// Family (6): in/out adjacency.
    fn check_adjacency(&self) -> Result<(), String> {
        let mut out_reference: imbl::HashMap<NodeId, Vec<AdjacencyEdge>> = imbl::HashMap::new();
        let mut in_reference: imbl::HashMap<NodeId, Vec<AdjacencyEdge>> = imbl::HashMap::new();
        for row in self.edge_store.alive.iter() {
            let Some(edge_id) = self.edge_id_for_row(crate::store::RowIndex::new(row)) else {
                return Err(format!("alive edge row {row} has no mapped external id"));
            };
            let Some(label) = self.edge_store.label.get(row as usize).cloned() else {
                return Err(format!("alive edge row {row} has no label column entry"));
            };
            let Some(source) = self.edge_store.source.get(row as usize).copied() else {
                return Err(format!("alive edge row {row} has no source column entry"));
            };
            let Some(target) = self.edge_store.target.get(row as usize).copied() else {
                return Err(format!("alive edge row {row} has no target column entry"));
            };
            out_reference
                .entry(source)
                .or_default()
                .push(AdjacencyEdge {
                    label: label.clone(),
                    neighbor: target,
                    edge_id,
                });
            in_reference.entry(target).or_default().push(AdjacencyEdge {
                label,
                neighbor: source,
                edge_id,
            });
        }
        compare_adjacency("outgoing", &self.adjacency_out, &out_reference)?;
        compare_adjacency("incoming", &self.adjacency_in, &in_reference)?;
        Ok(())
    }
}

/// Compare a maintained `IStr`-keyed bitmap index against a re-derivation,
/// failing on any key difference, bitmap difference, or empty maintained
/// bucket.
fn compare_bitmap_index(
    name: &str,
    maintained: &imbl::HashMap<IStr, RoaringBitmap>,
    reference: &imbl::HashMap<IStr, RoaringBitmap>,
) -> Result<(), String> {
    for (label, bitmap) in maintained {
        if bitmap.is_empty() {
            return Err(format!(
                "{name}: key {label} holds a present-but-empty bitmap"
            ));
        }
        match reference.get(label) {
            None => {
                return Err(format!(
                    "{name}: key {label} is maintained but not re-derived (stale rows)"
                ));
            }
            Some(expected) if expected != bitmap => {
                return Err(format!(
                    "{name}: key {label} bitmap drifted (maintained {:?}, reference {:?})",
                    bitmap.iter().collect::<Vec<_>>(),
                    expected.iter().collect::<Vec<_>>()
                ));
            }
            Some(_) => {}
        }
    }
    for label in reference.keys() {
        if !maintained.contains_key(label) {
            return Err(format!(
                "{name}: key {label} is re-derived but missing from the maintained index"
            ));
        }
    }
    Ok(())
}

/// Compare a maintained adjacency map against a re-derivation. The maintained
/// entry is sorted by `(label, neighbor, edge_id)`; the reference is sorted to
/// match before comparison so parallel edges and ordering are both checked.
fn compare_adjacency(
    direction: &str,
    maintained: &imbl::HashMap<NodeId, crate::adjacency::AdjacencyEntry>,
    reference: &imbl::HashMap<NodeId, Vec<AdjacencyEdge>>,
) -> Result<(), String> {
    for (node, entry) in maintained {
        if entry.is_empty() {
            return Err(format!(
                "{direction} adjacency: node {node} holds a present-but-empty entry"
            ));
        }
        let maintained_edges: Vec<AdjacencyEdge> = entry.iter().cloned().collect();
        match reference.get(node) {
            None => {
                return Err(format!(
                    "{direction} adjacency: node {node} is maintained but has no alive edges \
                     (dangling adjacency)"
                ));
            }
            Some(expected) => {
                let mut expected_sorted = expected.clone();
                expected_sorted.sort_by_key(adjacency_sort_key);
                if maintained_edges != expected_sorted {
                    return Err(format!(
                        "{direction} adjacency: node {node} drifted (maintained {maintained_edges:?}, \
                         reference {expected_sorted:?})"
                    ));
                }
            }
        }
    }
    for node in reference.keys() {
        if !maintained.contains_key(node) {
            return Err(format!(
                "{direction} adjacency: node {node} has alive edges but is missing from the \
                 maintained adjacency map"
            ));
        }
    }
    Ok(())
}

fn adjacency_sort_key(edge: &AdjacencyEdge) -> (IStr, NodeId, EdgeId) {
    (edge.label.clone(), edge.neighbor, edge.edge_id)
}

#[cfg(test)]
#[path = "consistency_tests.rs"]
mod tests;
