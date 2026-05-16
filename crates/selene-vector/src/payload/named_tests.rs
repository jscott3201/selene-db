use super::*;
use selene_core::NodeId;

#[test]
fn named_payload_prefix_roundtrips_and_legacy_routes_default() {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(1),
        vector: vec![1.0],
        max_layer: 0,
    }
    .encode()
    .unwrap();

    let named = split_named_payload(&encode_named_payload("episodes", payload.clone()).unwrap())
        .expect("named prefix decodes");
    assert_eq!(named.index_name, "episodes");
    assert_eq!(named.body, payload);

    let legacy = split_named_payload(&payload).expect("legacy payload decodes");
    assert_eq!(legacy.index_name, "default");
}

#[test]
fn lifecycle_payloads_roundtrip() {
    let create = VectorCreateIndexV1 {
        kind: "hnsw".to_owned(),
        config: vec![1, 2, 3],
    };
    let decoded = VectorCreateIndexV1::decode(&create.encode().unwrap()).unwrap();
    assert_eq!(decoded, create);
    assert!(matches!(
        decode_lifecycle_event(&create.encode().unwrap()).unwrap(),
        LifecycleEventKind::Create(decoded) if decoded == create
    ));

    assert!(matches!(
        decode_lifecycle_event(&VectorDropIndexV1.encode()).unwrap(),
        LifecycleEventKind::Drop(VectorDropIndexV1)
    ));
}

#[test]
fn named_payload_rejects_truncated_prefix() {
    let err =
        split_named_payload(&[NAMED_PAYLOAD_VERSION, 4]).expect_err("truncated prefix rejected");
    assert!(matches!(
        err,
        VectorError::InvalidPayload { reason } if reason.contains("name length")
    ));

    let err = split_named_payload(&[0x02]).expect_err("unknown prefix rejected");
    assert!(matches!(
        err,
        VectorError::InvalidPayload { reason } if reason.contains("unknown vector event prefix")
    ));
}
