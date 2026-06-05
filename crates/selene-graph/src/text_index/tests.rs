use selene_core::{GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, intern};

use super::*;
use crate::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn text_index_matches_exact_bm25_ranking() {
    let graph = SharedGraph::new(GraphId::new(433_001));
    let doc = istr("TextIndexedDoc");
    let other = istr("OtherDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("graph memory graph retrieval"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("vector retrieval retrieval"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("graph search"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("unmatched corpus document"))),
            )
            .unwrap();
        mutator
            .create_node(LabelSet::single(doc.clone()), props(&body, Value::Int(7)))
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(other),
                props(&body, Value::String(istr("graph retrieval"))),
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
    let doc = istr("TextIndexedMutableDoc");
    let body = istr("body");
    let stale;
    let fresh;
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        stale = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("stale memory"))),
            )
            .unwrap();
        fresh = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("fresh memory"))),
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
                PropertyDiff::new([(body.clone(), Value::String(istr("fresh updated")))], [])
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
    let doc = istr("TextStatsDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("agent graph graph"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("agent vector memory"))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("!!!"))),
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
    assert!(usage.estimated_index_bytes >= usage.posting_bytes);
}

#[test]
fn text_index_empty_query_and_zero_k_are_empty() {
    let graph = SharedGraph::new(GraphId::new(433_004));
    let doc = istr("TextIndexedEmptyDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("graph memory"))),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let index = graph.build_text_index(&doc, &body).unwrap();

    assert!(index.search("!!!", 10).is_empty());
    assert!(index.search("graph", 0).is_empty());
}

#[test]
fn shared_graph_indexed_text_search_matches_exact() {
    let graph = SharedGraph::new(GraphId::new(433_005));
    let doc = istr("TextSharedIndexedDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("agentic graph retrieval"))),
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
