//! Validators for the `QUNT` quantized snapshot overlay section.

use crate::quantize::linalg;
use crate::quantize::{QuantMethod, QuantizedStore};
use crate::{DistanceMetric, HnswConfig, PqParams, VectorError};

use super::super::{QUNT, decode_failed, expected_component_len, node_count_usize};
use super::QuntBodyV1;

pub(super) fn validate_qunt_body(body: &QuntBodyV1) -> Result<(), VectorError> {
    QuantMethod::from_wire(body.method)?;
    if body.dimensions == 0 {
        return Err(decode_failed(
            QUNT,
            "QUNT dimensions must be greater than zero",
        ));
    }
    let dim = usize::from(body.dimensions);
    let node_count = node_count_usize(body.node_count, QUNT)?;
    match &body.store {
        QuantizedStore::Sq8(store) => validate_sq8_store(store, dim, node_count, body.dimensions)?,
        QuantizedStore::Pq(store) => validate_pq_store(store, dim, node_count)?,
    }
    Ok(())
}

pub(super) fn validate_qunt_for_graph(
    body: &QuntBodyV1,
    graph: &crate::hnsw::HnswGraph,
    config: &HnswConfig,
) -> Result<QuantMethod, VectorError> {
    let method = QuantMethod::from_wire(body.method)?;
    if method != body.store.method() {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT method byte {} disagrees with store method {}",
                method.to_wire(),
                body.store.method().to_wire()
            ),
        ));
    }
    if usize::from(body.dimensions) != graph.dimensions() {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT dimension mismatch: {} != {}",
                body.dimensions,
                graph.dimensions()
            ),
        ));
    }
    let node_count = node_count_usize(body.node_count, QUNT)?;
    if node_count > graph.len() {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT node_count {} exceeds graph node count {}",
                body.node_count,
                graph.len()
            ),
        ));
    }
    validate_store_for_config(&body.store, config)?;
    Ok(method)
}

fn validate_sq8_store(
    store: &crate::quantize::QuantizedStoreSq8,
    dim: usize,
    node_count: usize,
    dimensions: u16,
) -> Result<(), VectorError> {
    let expected_len = expected_component_len(
        u32::try_from(node_count).map_err(|_| decode_failed(QUNT, "QUNT node_count overflow"))?,
        dimensions,
        QUNT,
    )?;
    if store.range.min.len() != dim || store.range.max.len() != dim {
        return Err(decode_failed(
            QUNT,
            format!("QUNT range length disagrees with dimensions {dim}"),
        ));
    }
    if store.codes.len() != expected_len {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT codes length {} disagrees with expected {expected_len}",
                store.codes.len()
            ),
        ));
    }
    if store.approx_norms.len() != node_count {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT approx_norms length {} disagrees with node_count {node_count}",
                store.approx_norms.len()
            ),
        ));
    }
    validate_ranges(&store.range.min, &store.range.max)?;
    validate_norms(Some(&store.approx_norms), node_count, true)?;
    Ok(())
}

fn validate_pq_store(
    store: &crate::quantize::QuantizedStorePq,
    dim: usize,
    node_count: usize,
) -> Result<(), VectorError> {
    let m = usize::try_from(store.codebook.m_subspaces)
        .map_err(|_| decode_failed(QUNT, "QUNT PQ m_subspaces overflow"))?;
    let k = usize::try_from(store.codebook.k_centroids)
        .map_err(|_| decode_failed(QUNT, "QUNT PQ k_centroids overflow"))?;
    let subdim = usize::try_from(store.codebook.subspace_dim)
        .map_err(|_| decode_failed(QUNT, "QUNT PQ subspace_dim overflow"))?;
    if m == 0 || k == 0 || subdim == 0 {
        return Err(decode_failed(
            QUNT,
            "QUNT PQ m_subspaces, k_centroids, and subspace_dim must be greater than zero",
        ));
    }
    if m.checked_mul(subdim) != Some(dim) {
        return Err(decode_failed(
            QUNT,
            format!("QUNT PQ m_subspaces * subspace_dim disagrees with dimensions {dim}"),
        ));
    }
    if store.codebook.k_centroids != PqParams::K_CENTROIDS_V1 {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT PQ k_centroids {} is not supported",
                store.codebook.k_centroids
            ),
        ));
    }
    let expected_codebook = m
        .checked_mul(k)
        .and_then(|value| value.checked_mul(subdim))
        .ok_or_else(|| decode_failed(QUNT, "QUNT PQ codebook length overflow"))?;
    if store.codebook.centroids.len() != expected_codebook {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT PQ codebook length {} disagrees with expected {expected_codebook}",
                store.codebook.centroids.len()
            ),
        ));
    }
    let expected_codes = node_count
        .checked_mul(m)
        .ok_or_else(|| decode_failed(QUNT, "QUNT PQ codes length overflow"))?;
    if store.codes.len() != expected_codes {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT PQ codes length {} disagrees with expected {expected_codes}",
                store.codes.len()
            ),
        ));
    }
    for (index, value) in store.codebook.centroids.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(decode_failed(
                QUNT,
                format!("non-finite QUNT PQ codebook component at index {index}: {value}"),
            ));
        }
    }
    validate_rotation(store.codebook.rotation.as_deref(), dim)?;
    validate_norms(store.approx_norms.as_deref(), node_count, false)?;
    Ok(())
}

fn validate_rotation(rotation: Option<&[f32]>, dim: usize) -> Result<(), VectorError> {
    let Some(rotation) = rotation else {
        return Ok(());
    };
    let expected = dim
        .checked_mul(dim)
        .ok_or_else(|| decode_failed(QUNT, "QUNT PQ OPQ rotation length overflow"))?;
    if rotation.len() != expected {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT PQ OPQ rotation length {} disagrees with dim^2 {expected}",
                rotation.len()
            ),
        ));
    }
    for (index, value) in rotation.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(decode_failed(
                QUNT,
                format!("non-finite QUNT PQ OPQ rotation component at index {index}: {value}"),
            ));
        }
    }
    if !linalg::is_orthonormal(rotation, dim, 1.0e-5) {
        return Err(decode_failed(
            QUNT,
            "QUNT PQ OPQ rotation is not orthonormal",
        ));
    }
    Ok(())
}

fn validate_store_for_config(
    store: &QuantizedStore,
    config: &HnswConfig,
) -> Result<(), VectorError> {
    if let QuantizedStore::Pq(store) = store {
        let params = PqParams::resolve(config.dim, config.quantization.pq);
        if store.codebook.m_subspaces as usize != params.m_subspaces {
            return Err(decode_failed(
                QUNT,
                "QUNT PQ m_subspaces disagrees with provider config",
            ));
        }
        if store.codebook.k_centroids != params.k_centroids {
            return Err(decode_failed(
                QUNT,
                "QUNT PQ k_centroids disagrees with provider config",
            ));
        }
        if store.codebook.dim() != config.dim {
            return Err(decode_failed(
                QUNT,
                "QUNT PQ dimensions disagree with provider config",
            ));
        }
        if !params.use_opq && store.codebook.rotation.is_some() {
            return Err(decode_failed(
                QUNT,
                "QUNT PQ OPQ rotation present while provider config disables OPQ",
            ));
        }
        if config.metric == DistanceMetric::Cosine && store.approx_norms.is_none() {
            return Err(decode_failed(
                QUNT,
                "QUNT PQ cosine store requires approx_norms",
            ));
        }
    }
    Ok(())
}

fn validate_ranges(min: &[f32], max: &[f32]) -> Result<(), VectorError> {
    for (index, (min, max)) in min.iter().copied().zip(max.iter().copied()).enumerate() {
        if !min.is_finite() || !max.is_finite() {
            return Err(decode_failed(
                QUNT,
                format!("non-finite QUNT range at coordinate {index}: {min}, {max}"),
            ));
        }
        if min > max {
            return Err(decode_failed(
                QUNT,
                format!("inverted QUNT range at coordinate {index}: {min} > {max}"),
            ));
        }
    }
    Ok(())
}

fn validate_norms(
    norms: Option<&[f32]>,
    node_count: usize,
    required: bool,
) -> Result<(), VectorError> {
    let Some(norms) = norms else {
        if required {
            return Err(decode_failed(QUNT, "QUNT approx_norms are required"));
        }
        return Ok(());
    };
    if norms.len() != node_count {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT approx_norms length {} disagrees with node_count {node_count}",
                norms.len()
            ),
        ));
    }
    for (index, norm) in norms.iter().copied().enumerate() {
        if !norm.is_finite() {
            return Err(decode_failed(
                QUNT,
                format!("non-finite QUNT approx_norm at index {index}: {norm}"),
            ));
        }
        if norm < 0.0 {
            return Err(decode_failed(
                QUNT,
                format!("negative QUNT approx_norm at index {index}: {norm}"),
            ));
        }
    }
    Ok(())
}
