//! `selene.create_vector_index` native built-in.
//!
//! Mutation-tier procedure creating a vector index. Every write routes through
//! [`MutationContext::mutator`] → `Mutator::create_vector_index_named`, which
//! emits `SchemaChange::VectorIndexCreated` through the single mutation funnel
//! (Hard Rule 11). It never bypasses the funnel and never re-enters
//! `begin_write`.

use selene_core::{HnswIndexConfig, IStr, IvfIndexConfig, Value, VectorMetric};
use selene_graph::{GraphError, VectorIndexConfig, VectorIndexKind};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::unit_result;
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, MutationContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.create_vector_index";

static CREATE_VECTOR_INDEX_PARAMS: [StaticParameter; 9] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Vector property."),
    StaticParameter::new("dimension", GqlType::Integer, false)
        .with_description("Required vector dimensionality."),
    StaticParameter::new("kind", GqlType::String, false)
        .with_description("Vector index algorithm kind.")
        .with_default_doc("flat")
        .with_default(ProcedureDefaultValue::String("flat")),
    StaticParameter::new("name", GqlType::String, true)
        .with_description("Optional catalog name.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
    StaticParameter::new("metric", GqlType::String, true)
        .with_description("ANN distance metric.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
    StaticParameter::new("hnsw_max_neighbors", GqlType::Integer, true)
        .with_description("Optional HNSW M fanout.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
    StaticParameter::new("hnsw_ef_construction", GqlType::Integer, true)
        .with_description("Optional HNSW construction beam width.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
    StaticParameter::new("ivf_target_centroids", GqlType::Integer, true)
        .with_description("Optional IVF target centroid count.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
];

static CREATE_VECTOR_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    CREATE_VECTOR_INDEX_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    CREATE_VECTOR_INDEX_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &mut MutationContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(3..=9).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 3 to 9 arguments")));
    }
    let label = string_arg(&args[0], "label")?;
    let property = string_arg(&args[1], "property")?;
    let dimension = dimension_arg(&args[2])?;
    let metric = args.get(5).map(metric_arg).transpose()?.flatten();
    let kind = kind_arg(args.get(3), metric)?;
    let name = args.get(4).map(name_arg).transpose()?.flatten();
    let hnsw_config = hnsw_config_arg(args.get(6), args.get(7))?;
    let ivf_config = ivf_config_arg(args.get(8))?;

    match ctx.mutator().create_vector_index_named_with_configs(
        label.clone(),
        property.clone(),
        kind,
        dimension,
        name,
        VectorIndexConfig::new(hnsw_config, ivf_config),
    ) {
        Ok(()) => Ok(unit_result()),
        Err(GraphError::VectorIndexAlreadyExists { .. }) => Err(invalid_arg(format!(
            "vector index for ({label}, {property}) already exists"
        ))),
        Err(GraphError::VectorIndexInvalidDimension { .. }) => Err(invalid_arg(
            "vector index dimension must be greater than zero",
        )),
        Err(GraphError::VectorIndexInvalidHnswConfig { reason, .. }) => Err(invalid_arg(format!(
            "invalid HNSW vector index config: {reason}"
        ))),
        Err(GraphError::VectorIndexInvalidIvfConfig { reason, .. }) => Err(invalid_arg(format!(
            "invalid IVF vector index config: {reason}"
        ))),
        Err(GraphError::VectorIndexValueRejected { observed, .. }) => Err(invalid_arg(format!(
            "existing nodes contain values incompatible with the requested vector index: {observed}"
        ))),
        Err(other) => Err(ProcedureError::Internal {
            detail: format!("unexpected graph error during vector index creation: {other}"),
        }),
    }
}

fn string_arg(value: &Value, name: &'static str) -> Result<IStr, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-empty STRING"
        )));
    };
    if value.as_str().is_empty() {
        return Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-empty STRING"
        )));
    }
    Ok(value.clone())
}

fn dimension_arg(value: &Value) -> Result<u32, ProcedureError> {
    let dimension = match value {
        Value::Int(value) => u32::try_from(*value).ok(),
        Value::Uint(value) => u32::try_from(*value).ok(),
        _ => None,
    }
    .ok_or_else(|| invalid_arg(format!("{PROC_NAME} dimension must be a positive INTEGER")))?;
    if dimension == 0 {
        return Err(invalid_arg(format!(
            "{PROC_NAME} dimension must be a positive INTEGER"
        )));
    }
    Ok(dimension)
}

fn kind_arg(
    value: Option<&Value>,
    metric: Option<VectorMetric>,
) -> Result<VectorIndexKind, ProcedureError> {
    let Some(value) = value else {
        return Ok(VectorIndexKind::Flat);
    };
    let raw = string_arg(value, "kind")?;
    match raw.as_str().to_ascii_lowercase().as_str() {
        "flat" => {
            if metric.is_some() {
                return Err(invalid_arg(format!(
                    "{PROC_NAME} metric is only valid for ANN vector indexes"
                )));
            }
            Ok(VectorIndexKind::Flat)
        }
        "hnsw" => Ok(match metric.unwrap_or(VectorMetric::SquaredEuclidean) {
            VectorMetric::SquaredEuclidean => VectorIndexKind::HnswSquaredEuclidean,
            VectorMetric::Cosine => VectorIndexKind::HnswCosine,
            VectorMetric::NegativeInnerProduct => VectorIndexKind::HnswNegativeInnerProduct,
        }),
        "ivf" => Ok(match metric.unwrap_or(VectorMetric::SquaredEuclidean) {
            VectorMetric::SquaredEuclidean => VectorIndexKind::IvfSquaredEuclidean,
            VectorMetric::Cosine => VectorIndexKind::IvfCosine,
            VectorMetric::NegativeInnerProduct => VectorIndexKind::IvfNegativeInnerProduct,
        }),
        other => Err(invalid_arg(format!(
            "unknown vector index kind '{other}'; expected flat, hnsw, or ivf"
        ))),
    }
}

fn name_arg(value: &Value) -> Result<Option<IStr>, ProcedureError> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) if !value.as_str().is_empty() => Ok(Some(value.clone())),
        Value::String(_) => Err(invalid_arg(format!(
            "{PROC_NAME} name must be NULL or a non-empty STRING"
        ))),
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} name must be NULL or a non-empty STRING"
        ))),
    }
}

fn metric_arg(value: &Value) -> Result<Option<VectorMetric>, ProcedureError> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) => parse_metric(value).map(Some),
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} metric must be NULL or a STRING"
        ))),
    }
}

fn hnsw_config_arg(
    max_neighbors: Option<&Value>,
    ef_construction: Option<&Value>,
) -> Result<Option<HnswIndexConfig>, ProcedureError> {
    let max_neighbors = max_neighbors
        .map(|value| optional_u16_arg(value, "hnsw_max_neighbors"))
        .transpose()?
        .flatten();
    let ef_construction = ef_construction
        .map(|value| optional_u16_arg(value, "hnsw_ef_construction"))
        .transpose()?
        .flatten();
    if max_neighbors.is_none() && ef_construction.is_none() {
        return Ok(None);
    }
    let default = HnswIndexConfig::default();
    Ok(Some(HnswIndexConfig::new(
        max_neighbors.unwrap_or(default.max_neighbors),
        ef_construction.unwrap_or(default.ef_construction),
    )))
}

fn ivf_config_arg(
    target_centroids: Option<&Value>,
) -> Result<Option<IvfIndexConfig>, ProcedureError> {
    target_centroids
        .map(|value| optional_u16_arg(value, "ivf_target_centroids"))
        .transpose()
        .map(|value| value.flatten().map(IvfIndexConfig::new))
}

fn optional_u16_arg(value: &Value, name: &'static str) -> Result<Option<u16>, ProcedureError> {
    match value {
        Value::Null => Ok(None),
        Value::Int(value) => u16::try_from(*value)
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| invalid_arg(format!("{PROC_NAME} {name} must be a positive INTEGER"))),
        Value::Uint(value) => u16::try_from(*value)
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| invalid_arg(format!("{PROC_NAME} {name} must be a positive INTEGER"))),
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be NULL or a positive INTEGER"
        ))),
    }
}

fn parse_metric(value: &IStr) -> Result<VectorMetric, ProcedureError> {
    let raw = value.as_str();
    match raw.to_ascii_lowercase().as_str() {
        "squared_euclidean" | "sq_l2" | "l2" | "euclidean" => Ok(VectorMetric::SquaredEuclidean),
        "cosine" => Ok(VectorMetric::Cosine),
        "negative_inner_product" | "inner_product" | "mips" | "dot" => {
            Ok(VectorMetric::NegativeInnerProduct)
        }
        _ => Err(invalid_arg(format!(
            "unknown vector metric '{raw}'; expected squared_euclidean, cosine, or negative_inner_product"
        ))),
    }
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
