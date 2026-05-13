//! Codec for the `POST` IVF posting-list section.

use rkyv::{Archive, Deserialize, Serialize};
use selene_core::NodeId;

use crate::VectorError;
use crate::ivf::posting::{PostingEntry, PostingList};

use super::{POST, decode_failed, encode_failed};

/// Magic prefix for version-1 `POST` section bodies.
pub(crate) const PAYLOAD_MAGIC_POST: [u8; 4] = *b"VPOS";

/// Archived body for the `POST` section.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) enum PostBodyV1 {
    /// No IVF vectors have been inserted.
    Empty,
    /// Raw rows buffered below the training threshold.
    Deferred {
        /// Raw rows that must survive snapshot recovery before training.
        unassigned_rows: Vec<(NodeId, Vec<f32>)>,
    },
    /// Trained posting lists.
    Trained {
        /// Total entry count across posting lists.
        node_count: u32,
        /// Posting lists in dense centroid order.
        posting_lists: Vec<PostingList>,
    },
}

pub(crate) fn encode_post(body: &PostBodyV1) -> Result<Vec<u8>, VectorError> {
    validate_post(body)?;
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body)
        .map_err(|error| encode_failed(POST, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_POST.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_POST);
    out.extend_from_slice(&archived);
    Ok(out)
}

pub(crate) fn decode_post(bytes: &[u8]) -> Result<PostBodyV1, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_POST.len()) else {
        return Err(decode_failed(POST, "POST magic mismatch"));
    };
    if magic != PAYLOAD_MAGIC_POST {
        return Err(decode_failed(POST, "POST magic mismatch"));
    }
    let decoded = rkyv::from_bytes::<PostBodyV1, rkyv::rancor::Error>(body)
        .map_err(|error| decode_failed(POST, format!("rkyv decode failed: {error}")))?;
    validate_post(&decoded)?;
    Ok(decoded)
}

fn validate_post(body: &PostBodyV1) -> Result<(), VectorError> {
    match body {
        PostBodyV1::Empty => Ok(()),
        PostBodyV1::Deferred { unassigned_rows } => {
            for (row_index, (node_id, vector)) in unassigned_rows.iter().enumerate() {
                if *node_id == NodeId::TOMBSTONE {
                    return Err(decode_failed(
                        POST,
                        format!("POST deferred row {row_index}: TOMBSTONE node id"),
                    ));
                }
                if vector.is_empty() {
                    return Err(decode_failed(
                        POST,
                        format!("POST deferred row {row_index}: vector must be non-empty"),
                    ));
                }
                for (index, value) in vector.iter().copied().enumerate() {
                    if !value.is_finite() {
                        return Err(decode_failed(
                            POST,
                            format!(
                                "non-finite POST deferred row {row_index} component {index}: {value}"
                            ),
                        ));
                    }
                }
            }
            Ok(())
        }
        PostBodyV1::Trained {
            node_count,
            posting_lists,
        } => validate_trained(*node_count, posting_lists),
    }
}

fn validate_trained(node_count: u32, posting_lists: &[PostingList]) -> Result<(), VectorError> {
    let mut observed = 0_u32;
    for (expected_id, list) in posting_lists.iter().enumerate() {
        if list.centroid_id as usize != expected_id {
            return Err(decode_failed(
                POST,
                "POST centroid ids must be dense and ordered",
            ));
        }
        for entry in &list.entries {
            validate_entry(entry)?;
            observed = observed
                .checked_add(1)
                .ok_or_else(|| decode_failed(POST, "POST node_count overflow"))?;
        }
    }
    if observed != node_count {
        return Err(decode_failed(
            POST,
            format!("POST node_count {node_count} != observed {observed}"),
        ));
    }
    Ok(())
}

fn validate_entry(entry: &PostingEntry) -> Result<(), VectorError> {
    if entry.node_id == NodeId::TOMBSTONE {
        return Err(decode_failed(POST, "POST entry TOMBSTONE node id"));
    }
    if entry.codes.is_empty() {
        return Err(decode_failed(POST, "POST entry codes must be non-empty"));
    }
    if let Some(norm) = entry.reconstructed_norm
        && (!norm.is_finite() || norm < 0.0)
    {
        return Err(decode_failed(
            POST,
            "POST reconstructed_norm must be finite and nonnegative",
        ));
    }
    Ok(())
}
