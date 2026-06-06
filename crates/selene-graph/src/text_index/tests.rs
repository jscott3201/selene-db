use selene_core::{
    CancellationChecker, CancellationToken, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff,
    PropertyMap, Value,
};

use super::*;
use crate::SharedGraph;
use crate::text_search::TextSearchError;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn text_index_matches_exact_bm25_ranking() {
    let graph = SharedGraph::new(GraphId::new(433_001));
    let doc = db_string("TextIndexedDoc");
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
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("unmatched corpus document"))),
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

    let snapshot = graph.read();
    let exact = snapshot
        .exact_text_search_nodes(&doc, &body, "Graph retrieval", 10)
        .unwrap();
    let index = snapshot.build_text_index(&doc, &body).unwrap();
    let indexed = index.search("Graph retrieval", 10);

    assert_eq!(indexed, exact);
    assert_eq!(
        indexed.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![NodeId::new(1), NodeId::new(2), NodeId::new(3)]
    );
}

#[test]
fn text_index_rebuild_observes_update_and_delete_visibility() {
    let graph = SharedGraph::new(GraphId::new(433_002));
    let doc = db_string("TextIndexedMutableDoc");
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

    let before = graph.build_text_index(&doc, &body).unwrap();
    assert_eq!(
        before
            .search("fresh", 10)
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
    let after_update = graph.build_text_index(&doc, &body).unwrap();
    assert_eq!(
        after_update
            .search("updated", 10)
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
    let after_delete = graph.build_text_index(&doc, &body).unwrap();
    assert_eq!(
        after_delete
            .search("fresh", 10)
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![fresh]
    );
}

#[test]
fn text_index_reports_stats_and_memory() {
    let graph = SharedGraph::new(GraphId::new(433_003));
    let doc = db_string("TextStatsDoc");
    let body = db_string("body");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("agent graph graph"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("agent vector memory"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("!!!"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::Bool(true)),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let index = graph.build_text_index(&doc, &body).unwrap();
    let stats = index.stats();
    let usage = index.memory_usage();

    assert_eq!(stats.indexed_rows, 2);
    assert_eq!(stats.documents, 2);
    assert_eq!(stats.distinct_terms, 4);
    assert_eq!(stats.postings, 5);
    assert_eq!(stats.total_document_len, 6);
    assert_eq!(usage.documents, stats.documents);
    assert_eq!(usage.distinct_terms, stats.distinct_terms);
    assert!(usage.document_term_bytes > 0);
    assert!(usage.estimated_index_bytes >= usage.posting_bytes);
}

#[test]
fn text_index_empty_query_and_zero_k_are_empty() {
    let graph = SharedGraph::new(GraphId::new(433_004));
    let doc = db_string("TextIndexedEmptyDoc");
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
    let index = graph.build_text_index(&doc, &body).unwrap();

    assert!(index.search("!!!", 10).is_empty());
    assert!(index.search("graph", 0).is_empty());
    assert!(
        index
            .search_candidates("graph", &[NodeId::new(1)], 0)
            .is_empty()
    );
    assert!(
        index
            .search_candidates("!!!", &[NodeId::new(1)], 10)
            .is_empty()
    );
    assert!(index.search_candidates("graph", &[], 10).is_empty());
}

#[test]
fn shared_graph_indexed_text_search_matches_exact() {
    let graph = SharedGraph::new(GraphId::new(433_005));
    let doc = db_string("TextSharedIndexedDoc");
    let body = db_string("body");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("agentic graph retrieval"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    assert_eq!(
        graph
            .indexed_text_search_nodes(&doc, &body, "graph retrieval", 10)
            .unwrap(),
        graph
            .exact_text_search_nodes(&doc, &body, "graph retrieval", 10)
            .unwrap()
    );
}

#[test]
fn text_index_candidate_search_matches_global_filter() {
    let graph = SharedGraph::new(GraphId::new(433_006));
    let doc = db_string("TextCandidateDoc");
    let body = db_string("body");
    let keep_a;
    let keep_b;
    let reject;
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        keep_a = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("graph memory graph"))),
            )
            .unwrap();
        reject = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("graph memory memory"))),
            )
            .unwrap();
        keep_b = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("graph retrieval"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let index = graph.build_text_index(&doc, &body).unwrap();
    let candidates = [keep_a, keep_b];
    let candidate_set = std::collections::HashSet::<_>::from(candidates);
    let filtered_global = index
        .search("graph memory", 10)
        .into_iter()
        .filter(|hit| candidate_set.contains(&hit.node_id))
        .collect::<Vec<_>>();

    let scoped = index.search_candidates("graph memory", &candidates, 10);

    assert_eq!(scoped, filtered_global);
    assert_eq!(
        scoped.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![keep_a, keep_b]
    );
    assert!(!scoped.iter().any(|hit| hit.node_id == reject));
}

#[test]
fn text_index_candidate_search_dedups_and_ignores_unindexed_nodes() {
    let graph = SharedGraph::new(GraphId::new(433_007));
    let doc = db_string("TextCandidateDedupDoc");
    let body = db_string("body");
    let indexed;
    let non_string;
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        indexed = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("needle current memory"))),
            )
            .unwrap();
        non_string = mutator
            .create_node(LabelSet::single(doc.clone()), props(&body, Value::Int(7)))
            .unwrap();
        txn.commit().unwrap();
    }
    let index = graph.build_text_index(&doc, &body).unwrap();

    let hits = index.search_candidates(
        "needle",
        &[indexed, indexed, non_string, NodeId::new(999_999)],
        10,
    );

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node_id, indexed);
}

#[test]
fn text_index_candidate_search_checked_observes_cancelled_token() {
    let graph = SharedGraph::new(GraphId::new(433_008));
    let doc = db_string("TextCandidateCancelDoc");
    let body = db_string("body");
    let indexed;
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        indexed = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(db_string("graph memory"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let index = graph.build_text_index(&doc, &body).unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let checker = CancellationChecker::new(Some(&token), None);

    let err = index
        .search_candidates_checked("graph", &[indexed], 10, checker)
        .expect_err("cancelled token should stop candidate search");

    assert!(matches!(err, TextSearchError::Cancelled));
}
