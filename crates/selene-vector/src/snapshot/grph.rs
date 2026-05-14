//! Codec for the `GRPH` HNSW-topology snapshot section.

use std::collections::HashMap;

use rkyv::{Archive, Deserialize, Serialize};
use roaring::RoaringBitmap;

use crate::hnsw::{HnswGraph, InternalIndex};
use crate::{HnswConfig, VectorError};

use super::{GRPH, decode_failed, encode_failed, max_layer_cap, metric_to_wire, node_count_usize};

/// Magic prefix for version-1 `GRPH` section bodies.
pub(crate) const PAYLOAD_MAGIC_GRPH: [u8; 4] = *b"VGRP";

/// Magic prefix for version-2 `GRPH` section bodies.
pub(crate) const PAYLOAD_MAGIC_GRPH_V2: [u8; 4] = *b"VGP2";

/// Archived header for the `GRPH` section.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct GrphHeaderV1 {
    /// Vector dimensions locked at graph construction.
    pub(crate) dimensions: u16,
    /// Optional InternalIndex of the entry point.
    pub(crate) entry_point: Option<InternalIndex>,
    /// Highest HNSW layer present in any node.
    pub(crate) max_layer: u8,
    /// Total node count.
    pub(crate) node_count: u32,
    /// `DistanceMetric` wire representation.
    pub(crate) metric: u8,
    /// `HnswConfig.m` neighbor density.
    pub(crate) m: u16,
}

/// Archived per-node topology entry.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct GrphNodeV1 {
    /// Source graph node ID as `NodeId::get()`.
    pub(crate) node_id: u64,
    /// Highest HNSW layer containing this node.
    pub(crate) max_layer: u8,
    /// Per-layer neighbor lists in InternalIndex order.
    pub(crate) neighbors: Vec<Vec<InternalIndex>>,
}

type GrphBodyV1 = (GrphHeaderV1, Vec<GrphNodeV1>);
type GrphBodyV2 = (GrphHeaderV1, Vec<GrphNodeV1>, Vec<InternalIndex>);

/// Decoded `GRPH` topology plus optional explicit liveness state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GrphBody {
    /// Version-shared topology header.
    pub(crate) header: GrphHeaderV1,
    /// Physical-slot topology nodes.
    pub(crate) nodes: Vec<GrphNodeV1>,
    /// Live physical slots. `None` means V1 all-alive semantics.
    pub(crate) live_indices: Option<Vec<InternalIndex>>,
}

pub(crate) fn encode_grph(graph: &HnswGraph, config: &HnswConfig) -> Result<Vec<u8>, VectorError> {
    let node_count = u32::try_from(graph.len())
        .map_err(|_| encode_failed(GRPH, "graph node count exceeds GRPH header range"))?;
    let dimensions = u16::try_from(config.dim)
        .map_err(|_| encode_failed(GRPH, "provider config dim exceeds GRPH header range"))?;
    let m = u16::try_from(config.m)
        .map_err(|_| encode_failed(GRPH, "provider config m exceeds GRPH header range"))?;
    let header = GrphHeaderV1 {
        dimensions,
        entry_point: graph.entry_point(),
        max_layer: graph.max_layer(),
        node_count,
        metric: metric_to_wire(config.metric),
        m,
    };
    let nodes = graph
        .iter_nodes()
        .map(|node| GrphNodeV1 {
            node_id: node.node_id.get(),
            max_layer: node.max_layer,
            neighbors: node.neighbors.iter().cloned().collect(),
        })
        .collect::<Vec<_>>();
    let live_indices = graph.live_indices().collect::<Vec<_>>();
    let all_alive = live_indices.len() == graph.len();
    let (magic, archived) = if all_alive {
        let body: GrphBodyV1 = (header, nodes);
        (
            PAYLOAD_MAGIC_GRPH,
            rkyv::to_bytes::<rkyv::rancor::Error>(&body)
                .map_err(|error| encode_failed(GRPH, error.to_string()))?,
        )
    } else {
        let body: GrphBodyV2 = (header, nodes, live_indices);
        (
            PAYLOAD_MAGIC_GRPH_V2,
            rkyv::to_bytes::<rkyv::rancor::Error>(&body)
                .map_err(|error| encode_failed(GRPH, error.to_string()))?,
        )
    };
    let mut out = Vec::with_capacity(magic.len() + archived.len());
    out.extend_from_slice(&magic);
    out.extend_from_slice(&archived);
    Ok(out)
}

pub(crate) fn decode_grph(bytes: &[u8]) -> Result<GrphBody, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_GRPH.len()) else {
        return Err(decode_failed(GRPH, "GRPH magic mismatch"));
    };
    if magic == PAYLOAD_MAGIC_GRPH {
        let (header, nodes) = rkyv::from_bytes::<GrphBodyV1, rkyv::rancor::Error>(body)
            .map_err(|error| decode_failed(GRPH, format!("rkyv decode failed: {error}")))?;
        validate_grph(&header, &nodes, None)?;
        return Ok(GrphBody {
            header,
            nodes,
            live_indices: None,
        });
    }
    if magic == PAYLOAD_MAGIC_GRPH_V2 {
        let (header, nodes, live_indices) =
            rkyv::from_bytes::<GrphBodyV2, rkyv::rancor::Error>(body)
                .map_err(|error| decode_failed(GRPH, format!("rkyv decode failed: {error}")))?;
        validate_grph(&header, &nodes, Some(&live_indices))?;
        return Ok(GrphBody {
            header,
            nodes,
            live_indices: Some(live_indices),
        });
    }

    Err(decode_failed(GRPH, "GRPH magic mismatch"))
}

fn validate_grph(
    header: &GrphHeaderV1,
    nodes: &[GrphNodeV1],
    live_indices: Option<&[InternalIndex]>,
) -> Result<(), VectorError> {
    let node_count = node_count_usize(header.node_count, GRPH)?;
    if node_count != nodes.len() {
        return Err(decode_failed(
            GRPH,
            format!(
                "node_count {} disagrees with {} nodes",
                header.node_count,
                nodes.len()
            ),
        ));
    }
    let mut seen = HashMap::with_capacity(node_count);
    for (idx, node) in nodes.iter().enumerate() {
        validate_node(idx, node, nodes, &mut seen)?;
    }
    let live = validate_live_indices(node_count, live_indices)?;
    validate_entry_point(header, nodes, &live)?;
    let observed_max = live
        .iter()
        .filter_map(|idx| nodes.get(idx as usize))
        .map(|node| node.max_layer)
        .max()
        .unwrap_or(0);
    if header.max_layer != observed_max {
        return Err(decode_failed(
            GRPH,
            format!(
                "header max_layer {} disagrees with node max_layer {observed_max}",
                header.max_layer
            ),
        ));
    }
    Ok(())
}

fn validate_live_indices(
    node_count: usize,
    live_indices: Option<&[InternalIndex]>,
) -> Result<RoaringBitmap, VectorError> {
    let mut live = RoaringBitmap::new();
    match live_indices {
        None => {
            for idx in 0..node_count {
                let idx = InternalIndex::try_from(idx)
                    .map_err(|_| decode_failed(GRPH, "node_count exceeds InternalIndex range"))?;
                live.insert(idx);
            }
        }
        Some(indices) => {
            for idx in indices {
                let slot = usize::try_from(*idx)
                    .map_err(|_| decode_failed(GRPH, "live index exceeds usize range"))?;
                if slot >= node_count {
                    return Err(decode_failed(
                        GRPH,
                        format!("live index {idx} out of bounds for {node_count} nodes"),
                    ));
                }
                if !live.insert(*idx) {
                    return Err(decode_failed(GRPH, format!("duplicate live index {idx}")));
                }
            }
        }
    }
    Ok(live)
}

fn validate_node(
    idx: usize,
    node: &GrphNodeV1,
    nodes: &[GrphNodeV1],
    seen: &mut HashMap<u64, usize>,
) -> Result<(), VectorError> {
    if node.node_id == 0 {
        return Err(decode_failed(GRPH, "node_id 0 is NodeId::TOMBSTONE"));
    }
    if node.max_layer > max_layer_cap() {
        return Err(decode_failed(
            GRPH,
            format!(
                "max_layer {} exceeds cap {}",
                node.max_layer,
                max_layer_cap()
            ),
        ));
    }
    if let Some(prev) = seen.insert(node.node_id, idx) {
        return Err(decode_failed(
            GRPH,
            format!(
                "duplicate node_id {} at indexes {prev}, {idx}",
                node.node_id
            ),
        ));
    }
    let expected_layers = usize::from(node.max_layer) + 1;
    if node.neighbors.len() != expected_layers {
        return Err(decode_failed(
            GRPH,
            format!(
                "node {idx} has {} neighbor layers, expected {expected_layers}",
                node.neighbors.len()
            ),
        ));
    }
    for (layer, neighbors) in node.neighbors.iter().enumerate() {
        for neighbor in neighbors {
            let neighbor_idx = usize::try_from(*neighbor)
                .map_err(|_| decode_failed(GRPH, "neighbor index exceeds usize range"))?;
            let Some(neighbor_node) = nodes.get(neighbor_idx) else {
                return Err(decode_failed(
                    GRPH,
                    format!("node {idx} layer {layer} neighbor {neighbor} out of bounds"),
                ));
            };
            if usize::from(neighbor_node.max_layer) < layer {
                return Err(decode_failed(
                    GRPH,
                    format!(
                        "node {idx} layer {layer} neighbor {neighbor} lacks layer {layer} (max_layer {})",
                        neighbor_node.max_layer
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_entry_point(
    header: &GrphHeaderV1,
    nodes: &[GrphNodeV1],
    live: &RoaringBitmap,
) -> Result<(), VectorError> {
    match (live.is_empty(), header.entry_point) {
        (true, None) => Ok(()),
        (true, Some(_)) => Err(decode_failed(GRPH, "empty graph must not have entry_point")),
        (false, None) => Err(decode_failed(GRPH, "live graph requires entry_point")),
        (false, Some(entry)) => {
            let entry_idx = usize::try_from(entry)
                .map_err(|_| decode_failed(GRPH, "entry_point exceeds usize range"))?;
            if !live.contains(entry) {
                return Err(decode_failed(GRPH, "entry_point is not live"));
            }
            let Some(entry_node) = nodes.get(entry_idx) else {
                return Err(decode_failed(GRPH, "entry_point out of bounds"));
            };
            if entry_node.max_layer != header.max_layer {
                return Err(decode_failed(
                    GRPH,
                    format!(
                        "entry_point max_layer {} disagrees with header max_layer {}",
                        entry_node.max_layer, header.max_layer
                    ),
                ));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_header(node_count: u32) -> GrphHeaderV1 {
        GrphHeaderV1 {
            dimensions: 2,
            entry_point: if node_count == 0 { None } else { Some(0) },
            max_layer: 0,
            node_count,
            metric: 0,
            m: 16,
        }
    }

    fn node(raw: u64, max_layer: u8, neighbors: Vec<Vec<InternalIndex>>) -> GrphNodeV1 {
        GrphNodeV1 {
            node_id: raw,
            max_layer,
            neighbors,
        }
    }

    fn raw_encode(body: GrphBodyV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&body).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_GRPH.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_GRPH);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn decode_rejects_wrong_magic() {
        let err = decode_grph(b"XXXX").expect_err("bad magic rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("magic"))
        );
    }

    #[test]
    fn decode_rejects_duplicate_node_id() {
        let bytes = raw_encode((
            valid_header(2),
            vec![node(1, 0, vec![vec![]]), node(1, 0, vec![vec![]])],
        ));
        let err = decode_grph(&bytes).expect_err("duplicate rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("duplicate"))
        );
    }

    #[test]
    fn decode_rejects_tombstone_node_id() {
        let bytes = raw_encode((valid_header(1), vec![node(0, 0, vec![vec![]])]));
        let err = decode_grph(&bytes).expect_err("tombstone rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("TOMBSTONE"))
        );
    }

    #[test]
    fn decode_rejects_out_of_bounds_neighbor() {
        let bytes = raw_encode((valid_header(1), vec![node(1, 0, vec![vec![1]])]));
        let err = decode_grph(&bytes).expect_err("oob neighbor rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("out of bounds"))
        );
    }

    #[test]
    fn decode_rejects_neighbor_layer_membership_violation() {
        let mut header = valid_header(2);
        header.max_layer = 1;
        let bytes = raw_encode((
            header,
            vec![node(1, 1, vec![vec![], vec![1]]), node(2, 0, vec![vec![]])],
        ));
        let err = decode_grph(&bytes).expect_err("layer violation rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("lacks layer"))
        );
    }

    #[test]
    fn decode_rejects_entry_point_invariants() {
        let mut header = valid_header(0);
        header.entry_point = Some(0);
        let err = decode_grph(&raw_encode((header, vec![]))).expect_err("entry rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("entry_point"))
        );
    }

    #[test]
    fn decode_rejects_max_layer_above_cap() {
        let mut header = valid_header(1);
        header.max_layer = 33;
        let err = decode_grph(&raw_encode((header, vec![node(1, 33, vec![vec![]; 34])])))
            .expect_err("max layer rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("exceeds cap"))
        );
    }
}
