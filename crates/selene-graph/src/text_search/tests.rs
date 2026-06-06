use std::time::{Duration, Instant};

use selene_core::{
    CancellationChecker, CancellationToken, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff,
    PropertyMap, Value,
};

use super::*;
use crate::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn exact_text_search_ranks_labelled_string_nodes() {
    let graph = SharedGraph::new(GraphId::new(431_001));
    let doc = db_string("TextDoc");
    let other = db_string("OtherDoc");
    let body = db_string("body");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(
                    &body,
                    Value::String(db_string("graph memory graph retrieval")),
                ),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(
                    &body,
                    Value::String(db_string("vector retrieval retrieval")),
                ),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("graph search"))),
            )
            .unwrap();
        mutator
            .create_node(LabelSet::single(doc.clone()), props(&body, Value::Int(7)))
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(other),
                props(&body, Value::String(db_string("graph retrieval"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let hits = graph
        .exact_text_search_nodes(&doc, &body, "Graph retrieval", 3)
        .unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1), NodeId::new(2), NodeId::new(3)]
    );
    assert!(hits[0].score > hits[1].score);
    assert!(hits[1].score > hits[2].score);
}

#[test]
fn exact_text_search_tokenizes_case_and_punctuation() {
    let graph = SharedGraph::new(GraphId::new(431_002));
    let doc = db_string("TextTokenDoc");
    let body = db_string("body");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(
                    &body,
                    Value::String(db_string("Agentic-memory, Graph_Retrieval!")),
                ),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let hits = graph
        .exact_text_search_nodes(&doc, &body, "agentic graph retrieval", 10)
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node_id, NodeId::new(1));
    assert!(hits[0].score > 0.0);
}

#[test]
fn exact_text_search_tracks_update_and_delete_visibility() {
    let graph = SharedGraph::new(GraphId::new(431_003));
    let doc = db_string("TextMutableDoc");
    let body = db_string("body");
    let stale;
    let fresh;
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        stale = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("stale memory"))),
            )
            .unwrap();
        fresh = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("fresh memory"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        graph
            .exact_text_search_nodes(&doc, &body, "fresh", 10)
            .unwrap()
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![fresh]
    );

    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .update_node(
                stale,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new(
                    [(body.clone(), Value::String(db_string("fresh updated")))],
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        graph
            .exact_text_search_nodes(&doc, &body, "updated", 10)
            .unwrap()
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![stale]
    );

    {
        let mut txn = graph.begin_write();
        txn.mutator().delete_node(stale).unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        graph
            .exact_text_search_nodes(&doc, &body, "fresh", 10)
            .unwrap()
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![fresh]
    );
}

#[test]
fn exact_text_search_empty_query_and_zero_k_are_empty() {
    let graph = SharedGraph::new(GraphId::new(431_004));
    let doc = db_string("TextEmptyDoc");
    let body = db_string("body");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("graph memory"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    assert!(
        graph
            .exact_text_search_nodes(&doc, &body, "!!!", 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        graph
            .exact_text_search_nodes(&doc, &body, "graph", 0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exact_text_search_checked_observes_cancelled_token_before_scan() {
    let graph = SharedGraph::new(GraphId::new(431_005));
    let doc = db_string("TextCancelDoc");
    let body = db_string("body");
    let token = CancellationToken::new();
    token.cancel();
    let checker = CancellationChecker::new(Some(&token), None);

    let err = graph
        .exact_text_search_nodes_checked(&doc, &body, "graph", 10, checker)
        .expect_err("cancelled token should stop search");

    assert!(matches!(err, TextSearchError::Cancelled));
}

#[test]
fn exact_text_search_checked_observes_elapsed_deadline_before_scan() {
    let graph = SharedGraph::new(GraphId::new(431_006));
    let doc = db_string("TextTimeoutDoc");
    let body = db_string("body");
    let checker = CancellationChecker::new(None, Some(Instant::now() - Duration::from_secs(1)));

    let err = graph
        .exact_text_search_nodes_checked(&doc, &body, "graph", 10, checker)
        .expect_err("expired deadline should stop search");

    assert!(matches!(err, TextSearchError::Timeout { .. }));
}
