//! Vector-index rebuild reporting.

use std::num::NonZeroUsize;

use selene_core::{HnswIndexConfig, IStr, IvfIndexConfig};

use super::{VectorIndexKind, VectorIndexMemoryUsage};

/// Policy for explicit vector-index maintenance runs.
///
/// The policy only controls derived index maintenance. It never changes graph
/// data, WAL contents, or vector-index registrations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VectorIndexMaintenancePolicy {
    /// Maximum recommended indexes to rebuild in one maintenance call.
    ///
    /// `None` rebuilds every index whose diagnostics currently recommend
    /// maintenance. A cap lets embedders amortize IVF retraining across
    /// foreground write-heavy periods while keeping reads free of rebuild work.
    pub max_indexes_per_run: Option<NonZeroUsize>,
}

impl VectorIndexMaintenancePolicy {
    /// Return the default recommended-index maintenance policy.
    #[must_use]
    pub const fn recommended() -> Self {
        Self {
            max_indexes_per_run: None,
        }
    }

    /// Return a policy capped to at most `max_indexes_per_run` rebuilds.
    #[must_use]
    pub const fn with_max_indexes_per_run(mut self, max_indexes_per_run: NonZeroUsize) -> Self {
        self.max_indexes_per_run = Some(max_indexes_per_run);
        self
    }
}

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
    /// IVF construction config for IVF indexes.
    pub ivf_config: Option<IvfIndexConfig>,
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
