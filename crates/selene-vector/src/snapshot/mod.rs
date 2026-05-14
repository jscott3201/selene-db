//! Snapshot section codecs for the selene-vector provider.

use std::collections::HashMap;
use std::sync::Arc;

use selene_core::NodeId;
use smallvec::SmallVec;

use crate::config::DistanceMetric;
use crate::hnsw::{HnswGraph, HnswNode, InternalIndex};
use crate::{HnswConfig, VectorError};

pub(crate) mod cqnt;
pub(crate) mod grph;
pub(crate) mod ipqb;
pub(crate) mod post;
pub(crate) mod qunt;
mod tags;
pub(crate) mod vecs;

pub(crate) use tags::{
    CQNT, DECLARED_SUB_TAGS, DECLARED_SUB_TAGS_IVF, GRPH, IPQB, IVFP, POST, QUNT, VECS, VECT,
    is_declared, is_declared_ivf,
};

const MAX_LAYER: u8 = 32;

pub(crate) fn assemble_graph(
    body: grph::GrphBody,
    vecs: vecs::VecsBodyV1,
    config: &HnswConfig,
) -> Result<HnswGraph, VectorError> {
    let header = body.header;
    let nodes = body.nodes;
    let live_indices = body.live_indices;
    validate_config(&header, config)?;
    validate_cross_section(&header, &vecs)?;

    let node_count = node_count_usize(header.node_count, GRPH)?;
    let dimensions = usize::from(header.dimensions);
    let mut graph = HnswGraph::with_capacity(header.dimensions, node_count);
    graph.entry_point = header.entry_point;
    graph.max_layer = header.max_layer;
    graph.node_id_to_idx = HashMap::with_capacity(node_count);
    let live_indices = live_indices.unwrap_or_else(|| {
        (0..node_count)
            .map(|idx| idx as InternalIndex)
            .collect::<Vec<_>>()
    });

    for (idx, node) in nodes.into_iter().enumerate() {
        let start = idx.checked_mul(dimensions).ok_or_else(|| {
            decode_failed(
                VECS,
                "node_count * dimensions overflow while assembling graph",
            )
        })?;
        let end = start.checked_add(dimensions).ok_or_else(|| {
            decode_failed(VECS, "vector slice offset overflow while assembling graph")
        })?;
        let vector: Arc<[f32]> = vecs.components[start..end].to_vec().into();
        let node_id = NodeId::new(node.node_id);
        let mut hnsw_node = HnswNode::new(node_id, vector, node.max_layer)?;
        let mut neighbors = SmallVec::with_capacity(node.neighbors.len());
        for layer_neighbors in node.neighbors {
            neighbors.push(layer_neighbors);
        }
        hnsw_node.neighbors = neighbors;
        graph.nodes.push(hnsw_node);
        let idx = InternalIndex::try_from(idx).map_err(|_| {
            decode_failed(
                GRPH,
                "node_count exceeds InternalIndex range while assembling graph",
            )
        })?;
        if live_indices.contains(&idx) {
            graph.mark_alive_idx(idx);
            graph.node_id_to_idx.insert(node_id, idx);
        }
    }

    Ok(graph)
}

pub(crate) fn validate_config(
    header: &grph::GrphHeaderV1,
    config: &HnswConfig,
) -> Result<(), VectorError> {
    let expected_dim = u16::try_from(config.dim)
        .map_err(|_| decode_failed(GRPH, "provider config dim exceeds snapshot header range"))?;
    let expected_m = u16::try_from(config.m)
        .map_err(|_| decode_failed(GRPH, "provider config m exceeds snapshot header range"))?;
    let expected_metric = metric_to_wire(config.metric);

    if header.dimensions != expected_dim {
        return Err(decode_failed(
            GRPH,
            format!(
                "snapshot vs provider config disagreement: dimensions {} != {}",
                header.dimensions, expected_dim
            ),
        ));
    }
    if header.metric != expected_metric {
        return Err(decode_failed(
            GRPH,
            format!(
                "snapshot vs provider config disagreement: metric {} != {}",
                header.metric, expected_metric
            ),
        ));
    }
    if header.m != expected_m {
        return Err(decode_failed(
            GRPH,
            format!(
                "snapshot vs provider config disagreement: m {} != {}",
                header.m, expected_m
            ),
        ));
    }

    Ok(())
}

fn validate_cross_section(
    header: &grph::GrphHeaderV1,
    vecs: &vecs::VecsBodyV1,
) -> Result<(), VectorError> {
    if header.dimensions != vecs.header.dimensions {
        return Err(decode_failed(
            VECS,
            format!(
                "GRPH/VECS dimension mismatch: {} != {}",
                header.dimensions, vecs.header.dimensions
            ),
        ));
    }
    if header.node_count != vecs.header.node_count {
        return Err(decode_failed(
            VECS,
            format!(
                "GRPH/VECS node_count mismatch: {} != {}",
                header.node_count, vecs.header.node_count
            ),
        ));
    }
    expected_component_len(header.node_count, header.dimensions, VECS)?;
    Ok(())
}

pub(crate) fn metric_to_wire(metric: DistanceMetric) -> u8 {
    match metric {
        DistanceMetric::Cosine => 0,
        DistanceMetric::L2 => 1,
        DistanceMetric::Dot => 2,
    }
}

pub(crate) fn expected_component_len(
    node_count: u32,
    dimensions: u16,
    sub_tag: selene_graph::SubTag,
) -> Result<usize, VectorError> {
    let node_count = node_count_usize(node_count, sub_tag)?;
    node_count
        .checked_mul(usize::from(dimensions))
        .ok_or_else(|| decode_failed(sub_tag, "node_count * dimensions overflow"))
}

pub(crate) fn node_count_usize(
    node_count: u32,
    sub_tag: selene_graph::SubTag,
) -> Result<usize, VectorError> {
    usize::try_from(node_count)
        .map_err(|_| decode_failed(sub_tag, "node_count exceeds usize range"))
}

pub(crate) fn decode_failed(
    sub_tag: selene_graph::SubTag,
    reason: impl Into<String>,
) -> VectorError {
    VectorError::SectionDecodeFailed {
        sub_tag,
        reason: reason.into(),
    }
}

pub(crate) fn encode_failed(
    sub_tag: selene_graph::SubTag,
    reason: impl Into<String>,
) -> VectorError {
    VectorError::SectionEncodeFailed {
        sub_tag,
        reason: reason.into(),
    }
}

pub(crate) fn max_layer_cap() -> u8 {
    MAX_LAYER
}
