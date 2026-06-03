//! Vector-index rebuild reporting.

use selene_core::{HnswIndexConfig, IStr};

use super::{VectorIndexKind, VectorIndexMemoryUsage};

/// One vector-index entry rebuilt by [`VectorIndexRebuildReport`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorIndexRebuildEntry {
    /// Indexed node label.
    pub label: IStr,
    /// Indexed node property.
    pub property: IStr,
    /// Optional explicit index catalog name.
    pub name: Option<IStr>,
    /// Rebuilt index algorithm kind.
    pub kind: VectorIndexKind,
    /// Rebuilt vector dimensionality.
    pub dimension: u32,
    /// HNSW construction config for HNSW indexes.
    pub hnsw_config: Option<HnswIndexConfig>,
    /// Memory and cardinality before the rebuild.
    pub before: VectorIndexMemoryUsage,
    /// Memory and cardinality after the rebuild.
    pub after: VectorIndexMemoryUsage,
}

/// Result returned after rebuilding all registered vector indexes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VectorIndexRebuildReport {
    /// Number of vector-index registrations rebuilt.
    pub indexes_rebuilt: usize,
    /// Per-index before/after memory accounting.
    pub entries: Vec<VectorIndexRebuildEntry>,
    /// HNSW entries removed by the rebuild, including stale deleted versions.
    pub reclaimed_hnsw_entries: usize,
    /// Stale HNSW deleted entries removed by the rebuild.
    pub reclaimed_hnsw_deleted_entries: usize,
    /// IVF entries removed by the rebuild, including stale deleted versions.
    pub reclaimed_ivf_entries: usize,
    /// Stale IVF deleted entries removed by the rebuild.
    pub reclaimed_ivf_deleted_entries: usize,
    /// Estimated index-owned bytes reclaimed by the rebuild.
    pub reclaimed_index_bytes: usize,
    /// Estimated reachable bytes reclaimed, including ANN vector components.
    pub reclaimed_reachable_bytes: usize,
}

impl VectorIndexRebuildReport {
    pub(crate) fn new(entries: Vec<VectorIndexRebuildEntry>) -> Self {
        let mut report = Self {
            indexes_rebuilt: entries.len(),
            entries,
            ..Self::default()
        };
        for entry in &report.entries {
            report.reclaimed_hnsw_entries = report.reclaimed_hnsw_entries.saturating_add(
                entry
                    .before
                    .hnsw_entries
                    .saturating_sub(entry.after.hnsw_entries),
            );
            report.reclaimed_hnsw_deleted_entries =
                report.reclaimed_hnsw_deleted_entries.saturating_add(
                    entry
                        .before
                        .hnsw_deleted_entries
                        .saturating_sub(entry.after.hnsw_deleted_entries),
                );
            report.reclaimed_ivf_entries = report.reclaimed_ivf_entries.saturating_add(
                entry
                    .before
                    .ivf_entries
                    .saturating_sub(entry.after.ivf_entries),
            );
            report.reclaimed_ivf_deleted_entries =
                report.reclaimed_ivf_deleted_entries.saturating_add(
                    entry
                        .before
                        .ivf_deleted_entries
                        .saturating_sub(entry.after.ivf_deleted_entries),
                );
            report.reclaimed_index_bytes = report.reclaimed_index_bytes.saturating_add(
                entry
                    .before
                    .estimated_index_bytes
                    .saturating_sub(entry.after.estimated_index_bytes),
            );
            report.reclaimed_reachable_bytes = report.reclaimed_reachable_bytes.saturating_add(
                entry
                    .before
                    .estimated_reachable_bytes
                    .saturating_sub(entry.after.estimated_reachable_bytes),
            );
        }
        report
    }
}
