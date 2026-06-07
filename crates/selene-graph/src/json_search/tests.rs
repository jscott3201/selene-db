use selene_core::{
    CancellationChecker, CancellationToken, GraphId, JsonPathSelector, JsonValue, LabelSet, NodeId,
    PropertyMap, Value, db_string,
};

use crate::{JsonPathContainmentCandidateOptions, SharedGraph};

fn label(value: &str) -> selene_core::DbString {
    db_string(value).expect("test string fits DB string cap")
}

fn json(value: serde_json::Value) -> Value {
    Value::Json(JsonValue::new(value).expect("JSON is valid"))
}

fn seed_docs(graph: &SharedGraph, doc: &selene_core::DbString, payload: &selene_core::DbString) {
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    for value in [
        json(serde_json::json!({
            "memory": {"facts": [{"title": "old"}, {"title": "current"}]}
        })),
        json(serde_json::json!({"memory": {"facts": [{"title": "only"}]}})),
        Value::String(label("not json")),
        json(serde_json::json!(["agent", {"memory": {"facts": [{"title": "current"}]}}])),
    ] {
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                PropertyMap::from_pairs([(payload.clone(), value)]).expect("properties are valid"),
            )
            .expect("node inserts");
    }
    txn.commit().expect("seed commits");
}

#[test]
fn exact_json_contains_nodes_matches_nested_candidates() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(1));
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    for value in [
        json(serde_json::json!({"memory": {"kind": "episodic", "score": 7}})),
        json(serde_json::json!({"memory": {"kind": "semantic"}})),
        Value::String(label("not json")),
        json(serde_json::json!(["agent", {"memory": {"kind": "episodic"}}])),
    ] {
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                PropertyMap::from_pairs([(payload.clone(), value)]).expect("properties are valid"),
            )
            .expect("node inserts");
    }
    txn.commit().expect("seed commits");

    let candidate = JsonValue::new(serde_json::json!({"memory": {"kind": "episodic"}})).unwrap();
    let hits = graph
        .exact_json_contains_nodes(&doc, &payload, &candidate, 10)
        .expect("search succeeds");

    assert_eq!(
        hits.into_iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1), NodeId::new(4)]
    );
}

#[test]
fn exact_json_contains_nodes_observes_zero_k_and_cancellation() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(2));
    let mut txn = graph.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(doc.clone()),
            PropertyMap::from_pairs([(
                payload.clone(),
                json(serde_json::json!({"kind": "memory"})),
            )])
            .expect("properties are valid"),
        )
        .expect("node inserts");
    txn.commit().expect("seed commits");
    let candidate = JsonValue::new(serde_json::json!({"kind": "memory"})).unwrap();

    assert!(
        graph
            .exact_json_contains_nodes(&doc, &payload, &candidate, 0)
            .expect("zero-k search succeeds")
            .is_empty()
    );

    let token = CancellationToken::new();
    token.cancel();
    let err = graph
        .exact_json_contains_nodes_checked(
            &doc,
            &payload,
            &candidate,
            10,
            CancellationChecker::new(Some(&token), None),
        )
        .expect_err("cancelled search reports cancellation");
    assert!(matches!(err, crate::JsonSearchError::Cancelled));
}

#[test]
fn exact_json_path_exists_nodes_matches_nested_paths() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(3));
    seed_docs(&graph, &doc, &payload);
    let path = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
        JsonPathSelector::Index(1),
        JsonPathSelector::Key(label("title")),
    ];

    let hits = graph
        .exact_json_path_exists_nodes(&doc, &payload, &path, 10)
        .expect("search succeeds");

    assert_eq!(
        hits.into_iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1)]
    );
}

#[test]
fn exact_json_path_exists_nodes_supports_reverse_array_index() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(4));
    seed_docs(&graph, &doc, &payload);
    let path = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
        JsonPathSelector::Index(-1),
        JsonPathSelector::Key(label("title")),
    ];

    let hits = graph
        .exact_json_path_exists_nodes(&doc, &payload, &path, 10)
        .expect("search succeeds");

    assert_eq!(
        hits.into_iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1), NodeId::new(2)]
    );
}

#[test]
fn exact_json_path_contains_nodes_matches_selected_subvalues() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(8));
    seed_docs(&graph, &doc, &payload);
    let path = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
    ];
    let candidate = JsonValue::new(serde_json::json!({"title": "current"})).unwrap();

    let hits = graph
        .exact_json_path_contains_nodes(&doc, &payload, &path, &candidate, 10)
        .expect("search succeeds");

    assert_eq!(
        hits.into_iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1)]
    );
}

#[test]
fn exact_json_path_contains_nodes_observes_zero_k_and_cancellation() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(9));
    seed_docs(&graph, &doc, &payload);
    let path = [JsonPathSelector::Key(label("memory"))];
    let candidate = JsonValue::new(serde_json::json!({"facts": []})).unwrap();

    assert!(
        graph
            .exact_json_path_contains_nodes(&doc, &payload, &path, &candidate, 0)
            .expect("zero-k search succeeds")
            .is_empty()
    );

    let token = CancellationToken::new();
    token.cancel();
    let err = graph
        .exact_json_path_contains_nodes_checked(
            &doc,
            &payload,
            &path,
            &candidate,
            10,
            CancellationChecker::new(Some(&token), None),
        )
        .expect_err("cancelled search reports cancellation");
    assert!(matches!(err, crate::JsonSearchError::Cancelled));
}

#[test]
fn exact_json_path_value_nodes_returns_selected_values() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(6));
    seed_docs(&graph, &doc, &payload);
    let path = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
        JsonPathSelector::Index(-1),
        JsonPathSelector::Key(label("title")),
    ];

    let hits = graph
        .exact_json_path_value_nodes(&doc, &payload, &path, 10)
        .expect("search succeeds");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].node_id, NodeId::new(1));
    assert_eq!(hits[0].value.as_serde(), &serde_json::json!("current"));
    assert_eq!(hits[1].node_id, NodeId::new(2));
    assert_eq!(hits[1].value.as_serde(), &serde_json::json!("only"));
}

#[test]
fn exact_json_path_value_nodes_observes_zero_k_and_cancellation() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(7));
    seed_docs(&graph, &doc, &payload);
    let path = [JsonPathSelector::Key(label("memory"))];

    assert!(
        graph
            .exact_json_path_value_nodes(&doc, &payload, &path, 0)
            .expect("zero-k search succeeds")
            .is_empty()
    );

    let token = CancellationToken::new();
    token.cancel();
    let err = graph
        .exact_json_path_value_nodes_checked(
            &doc,
            &payload,
            &path,
            10,
            CancellationChecker::new(Some(&token), None),
        )
        .expect_err("cancelled search reports cancellation");
    assert!(matches!(err, crate::JsonSearchError::Cancelled));
}

#[test]
fn exact_json_path_exists_nodes_observes_zero_k_and_cancellation() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(5));
    seed_docs(&graph, &doc, &payload);
    let path = [JsonPathSelector::Key(label("memory"))];

    assert!(
        graph
            .exact_json_path_exists_nodes(&doc, &payload, &path, 0)
            .expect("zero-k search succeeds")
            .is_empty()
    );

    let token = CancellationToken::new();
    token.cancel();
    let err = graph
        .exact_json_path_exists_nodes_checked(
            &doc,
            &payload,
            &path,
            10,
            CancellationChecker::new(Some(&token), None),
        )
        .expect_err("cancelled search reports cancellation");
    assert!(matches!(err, crate::JsonSearchError::Cancelled));
}

#[test]
fn exact_json_contains_candidate_nodes_filters_sorted_unique_candidates() {
    let doc = label("Doc");
    let other = label("Other");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(8));
    seed_docs(&graph, &doc, &payload);
    let mut txn = graph.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(other),
            PropertyMap::from_pairs([(
                payload.clone(),
                json(serde_json::json!({"memory": {"facts": [{"title": "current"}]}})),
            )])
            .expect("properties are valid"),
        )
        .expect("other-label node inserts");
    txn.commit().expect("other-label node commits");

    let candidate = JsonValue::new(serde_json::json!({"memory": {"facts": {"title": "current"}}}))
        .expect("candidate JSON parses");
    let hits = graph
        .exact_json_contains_candidate_nodes(
            &doc,
            &payload,
            &candidate,
            &[
                NodeId::new(5),
                NodeId::new(4),
                NodeId::new(1),
                NodeId::new(1),
                NodeId::new(999),
            ],
            10,
        )
        .expect("candidate search succeeds");

    assert_eq!(
        hits.into_iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1), NodeId::new(4)]
    );
}

#[test]
fn exact_json_path_candidate_nodes_match_global_path_semantics() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(9));
    seed_docs(&graph, &doc, &payload);

    let path_to_second_title = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
        JsonPathSelector::Index(1),
        JsonPathSelector::Key(label("title")),
    ];
    let path_to_facts = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
    ];
    let title_candidate =
        JsonValue::new(serde_json::json!({"title": "current"})).expect("candidate JSON parses");

    let exists_hits = graph
        .exact_json_path_exists_candidate_nodes(
            &doc,
            &payload,
            &path_to_second_title,
            &[NodeId::new(4), NodeId::new(2), NodeId::new(1)],
            10,
        )
        .expect("path-exists candidate search succeeds");
    assert_eq!(
        exists_hits
            .into_iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![NodeId::new(1)]
    );

    let contains_hits = graph
        .exact_json_path_contains_candidate_nodes(
            &doc,
            &payload,
            JsonPathContainmentCandidateOptions::new(
                &path_to_facts,
                &title_candidate,
                &[NodeId::new(4), NodeId::new(2), NodeId::new(1)],
                10,
            ),
        )
        .expect("path-containment candidate search succeeds");
    assert_eq!(
        contains_hits
            .into_iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![NodeId::new(1)]
    );
}

#[test]
fn exact_json_path_value_candidate_nodes_return_selected_values() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(10));
    seed_docs(&graph, &doc, &payload);
    let path = vec![
        JsonPathSelector::Key(label("memory")),
        JsonPathSelector::Key(label("facts")),
        JsonPathSelector::Index(-1),
        JsonPathSelector::Key(label("title")),
    ];

    let hits = graph
        .exact_json_path_value_candidate_nodes(
            &doc,
            &payload,
            &path,
            &[NodeId::new(2), NodeId::new(1), NodeId::new(4)],
            10,
        )
        .expect("path-value candidate search succeeds");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].node_id, NodeId::new(1));
    assert_eq!(hits[0].value.as_serde(), &serde_json::json!("current"));
    assert_eq!(hits[1].node_id, NodeId::new(2));
    assert_eq!(hits[1].value.as_serde(), &serde_json::json!("only"));
}

#[test]
fn exact_json_candidate_nodes_observe_zero_k_and_cancellation() {
    let doc = label("Doc");
    let payload = label("payload");
    let graph = SharedGraph::new(GraphId::new(11));
    seed_docs(&graph, &doc, &payload);
    let candidate = JsonValue::new(serde_json::json!({"memory": {}})).expect("candidate JSON");

    assert!(
        graph
            .exact_json_contains_candidate_nodes(&doc, &payload, &candidate, &[NodeId::new(1)], 0)
            .expect("zero-k candidate search succeeds")
            .is_empty()
    );

    let token = CancellationToken::new();
    token.cancel();
    let err = graph
        .exact_json_contains_candidate_nodes_checked(
            &doc,
            &payload,
            &candidate,
            &[NodeId::new(1)],
            10,
            CancellationChecker::new(Some(&token), None),
        )
        .expect_err("cancelled candidate search reports cancellation");
    assert!(matches!(err, crate::JsonSearchError::Cancelled));
}
