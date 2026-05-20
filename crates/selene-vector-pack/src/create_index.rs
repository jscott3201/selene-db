//! `vector.create_index` mutation procedure adapter.

use std::collections::HashMap;
use std::sync::Arc;

use selene_core::{Record, Value};
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult, RecordType};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{
    CreateIndexOutcome, DistanceMetric, HnswConfig, IvfConfig, NeighborSelectionConfig, PqParams,
    QuantMethod, QuantizationConfig, VectorCreateIndexV1, VectorError, encode_hnsw_config,
    encode_ivf_config,
};

use crate::{
    args::{expect_arity, required_string},
    error::invalid_argument,
    provider::{LIFECYCLE_PROVIDER_NAME, catalog_from_mutation, emit_payload_bytes},
    state::VectorPackState,
};

static CREATE_INDEX_NAME: [&str; 2] = ["vector", "create_index"];
const CREATE_INDEX_PROC: &str = "vector.create_index";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(CreateIndexProcedure { state })
}

struct CreateIndexProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for CreateIndexProcedure {
    fn name(&self) -> &'static [&'static str] {
        &CREATE_INDEX_NAME
    }

    fn description(&self) -> &'static str {
        "Create a vector index."
    }

    fn since_version(&self) -> &'static str {
        "1.1.0"
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("name", GqlType::String, false),
            parameter("kind", GqlType::String, false),
            parameter("config", GqlType::Record(RecordType::Open), false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalMutationProcedure for CreateIndexProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(CREATE_INDEX_PROC, args, 3)?;
        let name = required_string(CREATE_INDEX_PROC, args, 0, "name")?;
        let kind = canonical_kind(&required_string(CREATE_INDEX_PROC, args, 1, "kind")?)?;
        let fields = ConfigFields::new(&args[2])?;
        let catalog = catalog_from_mutation(ctx, CREATE_INDEX_PROC)?;

        match kind {
            IndexKind::Hnsw => {
                let config = parse_hnsw_config(&catalog, &fields)?;
                let outcome = catalog.create_hnsw_index(&name, config.clone())?;
                if outcome == CreateIndexOutcome::Created {
                    emit_create(
                        ctx,
                        &name,
                        "hnsw",
                        encode_hnsw_config(&config).map_err(lifecycle_config_error)?,
                    )?;
                }
            }
            IndexKind::Ivf => {
                let config = parse_ivf_config(&catalog, &fields)?;
                let outcome = catalog.create_ivf_index(&name, config)?;
                if outcome == CreateIndexOutcome::Created {
                    emit_create(
                        ctx,
                        &name,
                        "ivf",
                        encode_ivf_config(&config).map_err(lifecycle_config_error)?,
                    )?;
                }
            }
        };
        Ok(ProcedureResult { rows: Vec::new() })
    }
}

#[derive(Clone, Copy)]
enum IndexKind {
    Hnsw,
    Ivf,
}

fn canonical_kind(value: &str) -> Result<IndexKind, ProcedureError> {
    match value.to_ascii_lowercase().as_str() {
        "hnsw" => Ok(IndexKind::Hnsw),
        "ivf" => Ok(IndexKind::Ivf),
        other => Err(invalid_argument(format!(
            "{CREATE_INDEX_PROC}: kind must be 'hnsw' or 'ivf', got '{other}'"
        ))),
    }
}

struct ConfigFields<'a> {
    fields: HashMap<&'a str, &'a Value>,
}

impl<'a> ConfigFields<'a> {
    fn new(value: &'a Value) -> Result<Self, ProcedureError> {
        let Value::Record(record) = value else {
            return Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected config to be RECORD, got {value:?}"
            )));
        };
        let Record::Open(fields) = record.as_ref() else {
            return Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected config to be open RECORD"
            )));
        };
        let mut out = HashMap::with_capacity(fields.len());
        for (name, value) in fields {
            if out.insert(name.as_str(), value).is_some() {
                return Err(invalid_argument(format!(
                    "{CREATE_INDEX_PROC}: duplicate config field '{}'",
                    name.as_str()
                )));
            }
        }
        Ok(Self { fields: out })
    }

    fn reject_unknown(&self, allowed: &[&str]) -> Result<(), ProcedureError> {
        for name in self.fields.keys() {
            if !allowed.contains(name) {
                return Err(invalid_argument(format!(
                    "{CREATE_INDEX_PROC}: unknown config field '{name}'"
                )));
            }
        }
        Ok(())
    }

    fn get(&self, name: &str) -> Option<&Value> {
        self.fields.get(name).copied()
    }

    fn has_any(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.fields.contains_key(name))
    }
}

fn parse_hnsw_config(
    catalog: &crate::provider::RegistryCatalog,
    fields: &ConfigFields<'_>,
) -> Result<HnswConfig, ProcedureError> {
    fields.reject_unknown(HNSW_FIELDS)?;
    let mut config = catalog.hnsw_default_config()?;
    config.dim = get_usize(fields, "dim", config.dim)?;
    config.m = get_usize(fields, "m", config.m)?;
    config.ef_construction = get_usize(fields, "ef_construction", config.ef_construction)?;
    config.ef_search = get_usize(fields, "ef_search", config.ef_search)?;
    config.metric = get_metric(fields, "metric", config.metric)?;
    config.quantization = parse_quantization(fields, config.dim, config.quantization)?;
    config.neighbor_selection = NeighborSelectionConfig {
        extend_candidates: get_bool(
            fields,
            "neighbor_extend_candidates",
            config.neighbor_selection.extend_candidates,
        )?,
        keep_pruned_connections: get_bool(
            fields,
            "neighbor_keep_pruned_connections",
            config.neighbor_selection.keep_pruned_connections,
        )?,
    };
    config.validate().map_err(vector_config_error)?;
    Ok(config)
}

fn parse_ivf_config(
    catalog: &crate::provider::RegistryCatalog,
    fields: &ConfigFields<'_>,
) -> Result<IvfConfig, ProcedureError> {
    fields.reject_unknown(IVF_FIELDS)?;
    let mut config = catalog.ivf_default_config()?;
    config.dim = get_usize(fields, "dim", config.dim)?;
    config.k_coarse = get_u32(fields, "k_coarse", config.k_coarse)?;
    config.n_probe = get_u32(fields, "n_probe", config.n_probe)?;
    config.metric = get_metric(fields, "metric", config.metric)?;
    config.pq = parse_pq(fields, config.dim, config.pq)?;
    config.training_min_vectors =
        get_usize(fields, "training_min_vectors", config.training_min_vectors)?;
    config.validate().map_err(vector_config_error)?;
    Ok(config)
}

fn parse_quantization(
    fields: &ConfigFields<'_>,
    dim: usize,
    mut config: QuantizationConfig,
) -> Result<QuantizationConfig, ProcedureError> {
    config.enabled = get_bool(fields, "quantization_enabled", config.enabled)?;
    config.method = get_quant_method(fields, "quantization_method", config.method)?;
    config.rescore = get_bool(fields, "quantization_rescore", config.rescore)?;
    if fields.has_any(PQ_FIELDS) || config.method == QuantMethod::Pq {
        let pq = config.pq.unwrap_or_else(|| PqParams::default_for_dim(dim));
        config.pq = Some(parse_pq(fields, dim, pq)?);
    }
    Ok(config)
}

fn parse_pq(
    fields: &ConfigFields<'_>,
    _dim: usize,
    mut pq: PqParams,
) -> Result<PqParams, ProcedureError> {
    pq.m_subspaces = get_usize(fields, "pq_m_subspaces", pq.m_subspaces)?;
    pq.k_centroids = get_u32(fields, "pq_k_centroids", pq.k_centroids)?;
    pq.train_min_vectors = get_usize(fields, "pq_train_min_vectors", pq.train_min_vectors)?;
    pq.use_opq = get_bool(fields, "pq_use_opq", pq.use_opq)?;
    pq.use_polysemous = get_bool(fields, "pq_use_polysemous", pq.use_polysemous)?;
    pq.hamming_threshold_ratio = get_f32(
        fields,
        "pq_hamming_threshold_ratio",
        pq.hamming_threshold_ratio,
    )?;
    Ok(pq)
}

fn emit_create(
    ctx: &mut MutationContext<'_, '_>,
    name: &str,
    kind: &str,
    config: Vec<u8>,
) -> Result<(), ProcedureError> {
    let payload = VectorCreateIndexV1 {
        kind: kind.to_owned(),
        config,
    };
    let bytes = payload.encode().map_err(|error| ProcedureError::Internal {
        detail: format!("{CREATE_INDEX_PROC}: lifecycle payload encode failed: {error}"),
    })?;
    emit_payload_bytes(ctx, CREATE_INDEX_PROC, LIFECYCLE_PROVIDER_NAME, name, bytes)
}

fn get_usize(
    fields: &ConfigFields<'_>,
    name: &'static str,
    default: usize,
) -> Result<usize, ProcedureError> {
    fields
        .get(name)
        .map(|value| value_to_usize(name, value))
        .unwrap_or(Ok(default))
}

fn get_u32(
    fields: &ConfigFields<'_>,
    name: &'static str,
    default: u32,
) -> Result<u32, ProcedureError> {
    let value = get_usize(fields, name, default as usize)?;
    u32::try_from(value)
        .map_err(|_| invalid_argument(format!("{CREATE_INDEX_PROC}: {name} is too large")))
}

fn get_f32(
    fields: &ConfigFields<'_>,
    name: &'static str,
    default: f32,
) -> Result<f32, ProcedureError> {
    fields
        .get(name)
        .map(|value| match value {
            Value::Float(value) => Ok(*value as f32),
            Value::Float32(value) => Ok(*value),
            Value::Int(value) => Ok(*value as f32),
            Value::Uint(value) => Ok(*value as f32),
            other => Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected {name} to be FLOAT, got {other:?}"
            ))),
        })
        .unwrap_or(Ok(default))
}

fn get_bool(
    fields: &ConfigFields<'_>,
    name: &'static str,
    default: bool,
) -> Result<bool, ProcedureError> {
    fields
        .get(name)
        .map(|value| match value {
            Value::Bool(value) => Ok(*value),
            other => Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected {name} to be BOOL, got {other:?}"
            ))),
        })
        .unwrap_or(Ok(default))
}

fn get_metric(
    fields: &ConfigFields<'_>,
    name: &'static str,
    default: DistanceMetric,
) -> Result<DistanceMetric, ProcedureError> {
    fields
        .get(name)
        .map(|value| match string_value(value) {
            Some("cosine") => Ok(DistanceMetric::Cosine),
            Some("l2") => Ok(DistanceMetric::L2),
            Some("dot") => Ok(DistanceMetric::Dot),
            Some(other) => Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected {name} to be 'cosine', 'l2', or 'dot', got '{other}'"
            ))),
            None => Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected {name} to be STRING, got {value:?}"
            ))),
        })
        .unwrap_or(Ok(default))
}

fn get_quant_method(
    fields: &ConfigFields<'_>,
    name: &'static str,
    default: QuantMethod,
) -> Result<QuantMethod, ProcedureError> {
    fields
        .get(name)
        .map(|value| match string_value(value) {
            Some("sq8") => Ok(QuantMethod::Sq8),
            Some("pq") => Ok(QuantMethod::Pq),
            Some(other) => Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected {name} to be 'sq8' or 'pq', got '{other}'"
            ))),
            None => Err(invalid_argument(format!(
                "{CREATE_INDEX_PROC}: expected {name} to be STRING, got {value:?}"
            ))),
        })
        .unwrap_or(Ok(default))
}

fn value_to_usize(name: &'static str, value: &Value) -> Result<usize, ProcedureError> {
    match value {
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{CREATE_INDEX_PROC}: {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{CREATE_INDEX_PROC}: {name} must be non-negative"
        ))),
        Value::Uint(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{CREATE_INDEX_PROC}: {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{CREATE_INDEX_PROC}: expected {name} to be INTEGER, got {other:?}"
        ))),
    }
}

fn string_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value.as_str()),
        Value::ExternalString(value) => Some(value.as_ref()),
        _ => None,
    }
}

fn vector_config_error(error: VectorError) -> ProcedureError {
    invalid_argument(format!("{CREATE_INDEX_PROC}: {error}"))
}

fn lifecycle_config_error(error: VectorError) -> ProcedureError {
    ProcedureError::Internal {
        detail: format!("{CREATE_INDEX_PROC}: lifecycle config encode failed: {error}"),
    }
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    let parameter =
        ExternalParameter::new(name, ty, nullable).with_description("Procedure parameter.");
    if nullable {
        parameter.with_default_doc("NULL (use procedure default)")
    } else {
        parameter
    }
}

const PQ_FIELDS: &[&str] = &[
    "pq_m_subspaces",
    "pq_k_centroids",
    "pq_train_min_vectors",
    "pq_use_opq",
    "pq_use_polysemous",
    "pq_hamming_threshold_ratio",
];

const HNSW_FIELDS: &[&str] = &[
    "dim",
    "m",
    "ef_construction",
    "ef_search",
    "metric",
    "quantization_enabled",
    "quantization_method",
    "quantization_rescore",
    "neighbor_extend_candidates",
    "neighbor_keep_pruned_connections",
    "pq_m_subspaces",
    "pq_k_centroids",
    "pq_train_min_vectors",
    "pq_use_opq",
    "pq_use_polysemous",
    "pq_hamming_threshold_ratio",
];

const IVF_FIELDS: &[&str] = &[
    "dim",
    "k_coarse",
    "n_probe",
    "metric",
    "training_min_vectors",
    "pq_m_subspaces",
    "pq_k_centroids",
    "pq_train_min_vectors",
    "pq_use_opq",
    "pq_use_polysemous",
    "pq_hamming_threshold_ratio",
];
