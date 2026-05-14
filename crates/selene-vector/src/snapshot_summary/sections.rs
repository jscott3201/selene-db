//! Snapshot section rendering for vector snapshot fixtures.

use crate::snapshot::grph::decode_grph;
use crate::snapshot::qunt::decode_qunt;
use crate::snapshot::vecs::decode_vecs;
use crate::{QuantMethod, quantize::QuantizedStore};

/// Snapshot section bytes rendered into deterministic summaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorSectionsSummary {
    /// GRPH section bytes.
    pub grph: Vec<u8>,
    /// VECS section bytes.
    pub vecs: Vec<u8>,
    /// QUNT section bytes. Empty means quantization-disabled writer.
    pub qunt: Option<Vec<u8>>,
}

impl VectorSectionsSummary {
    /// Create a section summary from raw provider section bytes.
    #[must_use]
    pub fn new(grph: Vec<u8>, vecs: Vec<u8>, qunt: Option<Vec<u8>>) -> Self {
        Self { grph, vecs, qunt }
    }
}

pub(crate) fn render_sections(sections: &VectorSectionsSummary, out: &mut Vec<String>) {
    out.push(render_grph(&sections.grph));
    out.push(render_vecs(&sections.vecs));
    match &sections.qunt {
        Some(qunt) => out.push(render_qunt(qunt)),
        None => out.push("QUNT(<not-written>)".to_string()),
    }
}

fn render_grph(bytes: &[u8]) -> String {
    match decode_grph(bytes) {
        Ok(body) => {
            let head = body
                .nodes
                .iter()
                .take(4)
                .map(|node| {
                    let degrees = node
                        .neighbors
                        .iter()
                        .map(Vec::len)
                        .map(|degree| degree.to_string())
                        .collect::<Vec<_>>()
                        .join("/");
                    format!("n{}:L{}:[{}]", node.node_id, node.max_layer, degrees)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "GRPH(len={}, blake3_8={}, dim={}, nodes={}, entry_point={}, max_layer={}, head=[{}])",
                bytes.len(),
                blake3_8(bytes),
                body.header.dimensions,
                body.header.node_count,
                body.header
                    .entry_point
                    .map_or_else(|| "none".to_string(), |idx| format!("idx{idx}")),
                body.header.max_layer,
                head
            )
        }
        Err(err) => format!(
            "GRPH(len={}, blake3_8={}, decode_error={})",
            bytes.len(),
            blake3_8(bytes),
            super::errors::render_vector_error(&err)
        ),
    }
}

fn render_vecs(bytes: &[u8]) -> String {
    match decode_vecs(bytes) {
        Ok(body) => {
            let head = body
                .components
                .iter()
                .take(6)
                .map(|value| format!("{value:+.3}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "VECS(len={}, blake3_8={}, dim={}, nodes={}, components={}, head=[{}])",
                bytes.len(),
                blake3_8(bytes),
                body.header.dimensions,
                body.header.node_count,
                body.components.len(),
                head
            )
        }
        Err(err) => format!(
            "VECS(len={}, blake3_8={}, decode_error={})",
            bytes.len(),
            blake3_8(bytes),
            super::errors::render_vector_error(&err)
        ),
    }
}

fn render_qunt(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "QUNT(empty)".to_string();
    }
    match decode_qunt(bytes) {
        Ok(body) => match body.store {
            QuantizedStore::Sq8(store) => format!(
                "QUNT(len={}, blake3_8={}, method={:?}, dim={}, nodes={}, codes={}, ranges={}, norms={})",
                bytes.len(),
                blake3_8(bytes),
                method_token(body.method),
                body.dimensions,
                body.node_count,
                store.codes.len(),
                store.range.min.len(),
                store.approx_norms.len()
            ),
            QuantizedStore::Pq(store) => format!(
                "QUNT(len={}, blake3_8={}, method={:?}, dim={}, nodes={}, codebook_bytes={}, rotation_bytes={}, codes_bytes={}, norms={})",
                bytes.len(),
                blake3_8(bytes),
                method_token(body.method),
                body.dimensions,
                body.node_count,
                store.codebook.bytes_codebook(),
                store.codebook.bytes_rotation(),
                store.codes.len(),
                store.approx_norms.as_ref().map_or(0, Vec::len)
            ),
        },
        Err(err) => format!(
            "QUNT(len={}, blake3_8={}, decode_error={})",
            bytes.len(),
            blake3_8(bytes),
            super::errors::render_vector_error(&err)
        ),
    }
}

fn method_token(method: u8) -> &'static str {
    match QuantMethod::from_wire(method) {
        Ok(QuantMethod::Sq8) => "sq8",
        Ok(QuantMethod::Pq) => "pq",
        Err(_) => "unknown",
    }
}

fn blake3_8(bytes: &[u8]) -> String {
    let digest = blake3::hash(bytes);
    digest.as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
