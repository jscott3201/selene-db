//! Integration smoke tests for BRIEF-59 vector mutation payloads.

use selene_core::NodeId;
use selene_vector::{PAYLOAD_MAGIC, VectorError, VectorOp, VectorUpsertPayloadV1};

#[test]
fn roundtrip_insert_preserves_fields() {
    let payload = payload(VectorOp::Insert, vec![1.0, 2.0, 3.0]);

    let decoded = VectorUpsertPayloadV1::decode(&payload.encode().unwrap()).unwrap();

    assert_eq!(decoded, payload);
}

#[test]
fn roundtrip_delete_omits_vector() {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Delete,
        node_id: NodeId::new(7),
        vector: Vec::new(),
        max_layer: 0,
    };

    let decoded = VectorUpsertPayloadV1::decode(&payload.encode().unwrap()).unwrap();

    assert_eq!(decoded, payload);
}

#[test]
fn decode_rejects_wrong_magic() {
    let mut bytes = payload(VectorOp::Insert, vec![1.0]).encode().unwrap();
    bytes[0..4].copy_from_slice(b"XXXX");

    let err = VectorUpsertPayloadV1::decode(&bytes).expect_err("wrong magic is rejected");

    assert!(matches!(
        err,
        VectorError::InvalidPayload { reason } if reason.contains("magic")
    ));
}

#[test]
fn decode_rejects_truncated_bytes() {
    let err = VectorUpsertPayloadV1::decode(&PAYLOAD_MAGIC).expect_err("body is missing");

    assert!(matches!(err, VectorError::InvalidPayload { .. }));
}

#[test]
fn encode_validates_before_serializing() {
    let mut payload = payload(VectorOp::Insert, vec![1.0]);
    payload.max_layer = 99;

    let err = payload
        .encode()
        .expect_err("encode rejects pre-existing semantic violation");

    assert!(matches!(
        err,
        VectorError::InvalidPayload { reason } if reason.contains("max_layer")
    ));
}

#[test]
fn encode_rejects_insert_with_empty_vector() {
    let payload = payload(VectorOp::Insert, Vec::new());

    let err = payload
        .encode()
        .expect_err("encode rejects empty Insert vector");

    assert!(matches!(
        err,
        VectorError::InvalidPayload { reason } if reason.contains("Insert")
    ));
}

fn payload(op: VectorOp, vector: Vec<f32>) -> VectorUpsertPayloadV1 {
    VectorUpsertPayloadV1 {
        op,
        node_id: NodeId::new(7),
        vector,
        max_layer: 2,
    }
}
