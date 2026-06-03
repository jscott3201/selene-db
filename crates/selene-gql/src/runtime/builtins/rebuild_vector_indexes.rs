//! `selene.rebuild_vector_indexes` native built-in.
//!
//! Maintenance-tier procedure for rebuilding derived vector-index state from
//! primary graph values. This exposes the in-memory cleanup path to GQL without
//! treating it as schema DDL or data mutation.

use selene_core::{CancellationCause, IStr, Value, intern};
use selene_graph::{
    HnswIndexConfig, VectorIndexKind, VectorIndexMemoryUsage, VectorIndexRebuildEntry,
};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, MaintenanceContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult,
};

const PROC_NAME: &str = "selene.rebuild_vector_indexes";

static REBUILD_VECTOR_INDEXES_OUTPUTS: [StaticOutputColumn; 41] = [
    StaticOutputColumn::new("name", GqlType::String).with_description("Catalog index name."),
    StaticOutputColumn::new("label", GqlType::String).with_description("Indexed node label."),
    StaticOutputColumn::new("property", GqlType::String).with_description("Indexed property."),
    StaticOutputColumn::new("kind", GqlType::String).with_description("Vector index kind."),
    StaticOutputColumn::new("dimension", GqlType::Uint64)
        .with_description("Required vector dimensionality."),
    StaticOutputColumn::new("before_indexed_rows", GqlType::Uint64)
        .with_description("Live indexed row count before rebuild."),
    StaticOutputColumn::new("after_indexed_rows", GqlType::Uint64)
        .with_description("Live indexed row count after rebuild."),
    StaticOutputColumn::new("before_row_bitmap_bytes", GqlType::Uint64)
        .with_description("Estimated row-bitmap heap bytes before rebuild."),
    StaticOutputColumn::new("after_row_bitmap_bytes", GqlType::Uint64)
        .with_description("Estimated row-bitmap heap bytes after rebuild."),
    StaticOutputColumn::new("before_row_bitmap_serialized_bytes", GqlType::Uint64)
        .with_description("Serialized row-bitmap bytes before rebuild."),
    StaticOutputColumn::new("after_row_bitmap_serialized_bytes", GqlType::Uint64)
        .with_description("Serialized row-bitmap bytes after rebuild."),
    StaticOutputColumn::new("before_hnsw_index_bytes", GqlType::Uint64)
        .with_description("Estimated HNSW-owned heap bytes before rebuild."),
    StaticOutputColumn::new("after_hnsw_index_bytes", GqlType::Uint64)
        .with_description("Estimated HNSW-owned heap bytes after rebuild."),
    StaticOutputColumn::new("before_hnsw_referenced_vector_bytes", GqlType::Uint64)
        .with_description("Vector bytes reachable through HNSW before rebuild."),
    StaticOutputColumn::new("after_hnsw_referenced_vector_bytes", GqlType::Uint64)
        .with_description("Vector bytes reachable through HNSW after rebuild."),
    StaticOutputColumn::new("before_hnsw_entries", GqlType::Uint64)
        .with_description("Total HNSW entries before rebuild."),
    StaticOutputColumn::new("after_hnsw_entries", GqlType::Uint64)
        .with_description("Total HNSW entries after rebuild."),
    StaticOutputColumn::new("before_hnsw_live_entries", GqlType::Uint64)
        .with_description("Live HNSW entries before rebuild."),
    StaticOutputColumn::new("after_hnsw_live_entries", GqlType::Uint64)
        .with_description("Live HNSW entries after rebuild."),
    StaticOutputColumn::new("before_hnsw_deleted_entries", GqlType::Uint64)
        .with_description("Stale deleted HNSW entries before rebuild."),
    StaticOutputColumn::new("after_hnsw_deleted_entries", GqlType::Uint64)
        .with_description("Stale deleted HNSW entries after rebuild."),
    StaticOutputColumn::new("before_hnsw_link_count", GqlType::Uint64)
        .with_description("Stored directed HNSW links before rebuild."),
    StaticOutputColumn::new("after_hnsw_link_count", GqlType::Uint64)
        .with_description("Stored directed HNSW links after rebuild."),
    StaticOutputColumn::new("before_hnsw_level_zero_link_count", GqlType::Uint64)
        .with_description("Stored level-0 HNSW links before rebuild."),
    StaticOutputColumn::new("after_hnsw_level_zero_link_count", GqlType::Uint64)
        .with_description("Stored level-0 HNSW links after rebuild."),
    StaticOutputColumn::new("before_hnsw_upper_layer_link_count", GqlType::Uint64)
        .with_description("Stored upper-layer HNSW links before rebuild."),
    StaticOutputColumn::new("after_hnsw_upper_layer_link_count", GqlType::Uint64)
        .with_description("Stored upper-layer HNSW links after rebuild."),
    StaticOutputColumn::new("before_hnsw_max_layer_count", GqlType::Uint64)
        .with_description("Maximum HNSW layer count before rebuild."),
    StaticOutputColumn::new("after_hnsw_max_layer_count", GqlType::Uint64)
        .with_description("Maximum HNSW layer count after rebuild."),
    StaticOutputColumn::new("before_hnsw_max_links_per_layer", GqlType::Uint64)
        .with_description("Maximum HNSW links in one layer before rebuild."),
    StaticOutputColumn::new("after_hnsw_max_links_per_layer", GqlType::Uint64)
        .with_description("Maximum HNSW links in one layer after rebuild."),
    StaticOutputColumn::new(
        "before_hnsw_average_links_per_entry_basis_points",
        GqlType::Uint64,
    )
    .with_description("Average HNSW links per entry before rebuild scaled by 10,000."),
    StaticOutputColumn::new(
        "after_hnsw_average_links_per_entry_basis_points",
        GqlType::Uint64,
    )
    .with_description("Average HNSW links per entry after rebuild scaled by 10,000."),
    StaticOutputColumn::new("before_estimated_index_bytes", GqlType::Uint64)
        .with_description("Estimated index-owned bytes before rebuild."),
    StaticOutputColumn::new("after_estimated_index_bytes", GqlType::Uint64)
        .with_description("Estimated index-owned bytes after rebuild."),
    StaticOutputColumn::new("before_estimated_reachable_bytes", GqlType::Uint64)
        .with_description("Estimated reachable bytes before rebuild."),
    StaticOutputColumn::new("after_estimated_reachable_bytes", GqlType::Uint64)
        .with_description("Estimated reachable bytes after rebuild."),
    StaticOutputColumn::new("reclaimed_hnsw_entries", GqlType::Uint64)
        .with_description("HNSW entries reclaimed by this index rebuild."),
    StaticOutputColumn::new("reclaimed_hnsw_deleted_entries", GqlType::Uint64)
        .with_description("Stale HNSW deleted entries reclaimed by this index rebuild."),
    StaticOutputColumn::new("reclaimed_index_bytes", GqlType::Uint64)
        .with_description("Estimated index-owned bytes reclaimed by this index rebuild."),
    StaticOutputColumn::new("reclaimed_reachable_bytes", GqlType::Uint64)
        .with_description("Estimated reachable bytes reclaimed by this index rebuild."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    let params: [StaticParameter; 0] = [];
    params
        .into_iter()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    REBUILD_VECTOR_INDEXES_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &MaintenanceContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !args.is_empty() {
        return Err(ProcedureError::InvalidArgument {
            detail: format!("{PROC_NAME} expects zero arguments"),
        });
    }
    ctx.cancellation_checker()
        .check()
        .map_err(|cause| match cause {
            CancellationCause::Cancelled => ProcedureError::Cancelled,
            CancellationCause::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        })?;
    let report = ctx
        .rebuild_vector_indexes()
        .map_err(|source| ProcedureError::Internal {
            detail: format!("vector index rebuild failed: {source}"),
        })?;
    let rows = report
        .entries
        .into_iter()
        .map(RebuildRow::from_entry)
        .collect::<Vec<_>>();
    Ok(ProcedureResult {
        rows: rows
            .into_iter()
            .map(RebuildRow::into_values)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

struct RebuildRow {
    label: IStr,
    property: IStr,
    name: String,
    kind: String,
    dimension: u32,
    before: VectorIndexMemoryUsage,
    after: VectorIndexMemoryUsage,
}

impl RebuildRow {
    fn from_entry(entry: VectorIndexRebuildEntry) -> Self {
        let name = render_vector_index_name(
            entry.label.clone(),
            entry.property.clone(),
            entry.name.clone(),
        );
        let kind = render_vector_index_kind(entry.kind, entry.dimension, entry.hnsw_config);
        Self {
            label: entry.label,
            property: entry.property,
            name,
            kind,
            dimension: entry.dimension,
            before: entry.before,
            after: entry.after,
        }
    }

    fn into_values(self) -> Result<Vec<Value>, ProcedureError> {
        Ok(vec![
            string(&self.name)?,
            Value::String(self.label),
            Value::String(self.property),
            string(&self.kind)?,
            Value::Uint(u64::from(self.dimension)),
            Value::Uint(self.before.indexed_rows),
            Value::Uint(self.after.indexed_rows),
            bytes(self.before.row_bitmap_bytes),
            bytes(self.after.row_bitmap_bytes),
            bytes(self.before.row_bitmap_serialized_bytes),
            bytes(self.after.row_bitmap_serialized_bytes),
            bytes(self.before.hnsw_index_bytes),
            bytes(self.after.hnsw_index_bytes),
            bytes(self.before.hnsw_referenced_vector_bytes),
            bytes(self.after.hnsw_referenced_vector_bytes),
            bytes(self.before.hnsw_entries),
            bytes(self.after.hnsw_entries),
            bytes(self.before.hnsw_live_entries),
            bytes(self.after.hnsw_live_entries),
            bytes(self.before.hnsw_deleted_entries),
            bytes(self.after.hnsw_deleted_entries),
            bytes(self.before.hnsw_link_count),
            bytes(self.after.hnsw_link_count),
            bytes(self.before.hnsw_level_zero_link_count),
            bytes(self.after.hnsw_level_zero_link_count),
            bytes(self.before.hnsw_upper_layer_link_count),
            bytes(self.after.hnsw_upper_layer_link_count),
            bytes(self.before.hnsw_max_layer_count),
            bytes(self.after.hnsw_max_layer_count),
            bytes(self.before.hnsw_max_links_per_layer),
            bytes(self.after.hnsw_max_links_per_layer),
            bytes(self.before.hnsw_average_links_per_entry_basis_points),
            bytes(self.after.hnsw_average_links_per_entry_basis_points),
            bytes(self.before.estimated_index_bytes),
            bytes(self.after.estimated_index_bytes),
            bytes(self.before.estimated_reachable_bytes),
            bytes(self.after.estimated_reachable_bytes),
            bytes(
                self.before
                    .hnsw_entries
                    .saturating_sub(self.after.hnsw_entries),
            ),
            bytes(
                self.before
                    .hnsw_deleted_entries
                    .saturating_sub(self.after.hnsw_deleted_entries),
            ),
            bytes(
                self.before
                    .estimated_index_bytes
                    .saturating_sub(self.after.estimated_index_bytes),
            ),
            bytes(
                self.before
                    .estimated_reachable_bytes
                    .saturating_sub(self.after.estimated_reachable_bytes),
            ),
        ])
    }
}

fn render_vector_index_name(label: IStr, property: IStr, explicit: Option<IStr>) -> String {
    explicit
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|| {
            let label = label.as_str();
            let property = property.as_str();
            format!(
                "vidx:{}:{}:{}:{}",
                label.len(),
                label,
                property.len(),
                property
            )
        })
}

fn render_vector_index_kind(
    kind: VectorIndexKind,
    dimension: u32,
    hnsw_config: Option<HnswIndexConfig>,
) -> String {
    match kind {
        VectorIndexKind::Flat => format!("vector_flat({dimension})"),
        VectorIndexKind::HnswSquaredEuclidean => {
            render_hnsw_kind("vector_hnsw_squared_euclidean", dimension, hnsw_config)
        }
        VectorIndexKind::HnswCosine => {
            render_hnsw_kind("vector_hnsw_cosine", dimension, hnsw_config)
        }
        VectorIndexKind::HnswNegativeInnerProduct => {
            render_hnsw_kind("vector_hnsw_negative_inner_product", dimension, hnsw_config)
        }
    }
}

fn render_hnsw_kind(
    name: &'static str,
    dimension: u32,
    hnsw_config: Option<HnswIndexConfig>,
) -> String {
    let config = hnsw_config.unwrap_or_default();
    if config.is_default() {
        format!("{name}({dimension})")
    } else {
        format!(
            "{name}({dimension},m={},ef_construction={})",
            config.max_neighbors, config.ef_construction
        )
    }
}

fn bytes(value: usize) -> Value {
    Value::Uint(u64::try_from(value).unwrap_or(u64::MAX))
}

fn string(value: &str) -> Result<Value, ProcedureError> {
    intern(value)
        .map(Value::String)
        .map_err(|_err| ProcedureError::Internal {
            detail: "interner cap exhausted during selene.rebuild_vector_indexes".to_owned(),
        })
}
