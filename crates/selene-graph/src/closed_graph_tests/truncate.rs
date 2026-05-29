//! GG02 (closed-graph) behaviour of `IM_TRUNCATE` (BRIEF-150, audit Item 11 test #7).
//!
//! TRUNCATE removes the *instances* of a declared node/edge type; it does NOT
//! drop the type itself. So under a closed (GG02) graph it must commit cleanly
//! — the closed-graph validator treats the declarative truncate exactly like a
//! bulk `DETACH DELETE` — and the bound `GraphTypeDef` must survive: the graph
//! stays closed and a subsequent valid insert against the still-declared type
//! still commits.

use super::*;

#[test]
fn truncate_under_closed_graph_keeps_bound_type() {
    let shared = SharedGraph::builder(GraphId::new(150))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();

    // Seed two Person nodes joined by a declared KNOWS edge.
    let mut txn = shared.begin_write();
    let (alice, bob) = {
        let mut m = txn.mutator();
        let alice = m
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap();
        let bob = m
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Bob"))),
            )
            .unwrap();
        m.create_edge(istr("KNOWS"), alice, bob, prop("since", Value::Int(2020)))
            .unwrap();
        (alice, bob)
    };
    txn.commit().unwrap();
    assert_eq!(shared.read().node_count(), 2);
    assert_eq!(shared.read().edge_count(), 1);
    assert!(shared.is_closed());

    // TRUNCATE NODE TYPE :Person under the closed graph must commit: truncate
    // removes instances, it does not drop the declared type, so the validator
    // raises no graph-type violation (it treats the truncate variant like a
    // bulk DETACH DELETE).
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        m.truncate_node_type(istr("Person")).unwrap();
    }
    txn.commit()
        .expect("truncate under a closed graph commits — it removes instances, not the type");

    {
        let read = shared.read();
        assert_eq!(read.node_count(), 0, "all Person instances removed");
        assert_eq!(
            read.edge_count(),
            0,
            "incident KNOWS edge removed — no dangling edge"
        );
        assert!(!read.is_node_alive(alice));
        assert!(!read.is_node_alive(bob));
        assert!(
            read.nodes_with_label(&istr("Person")).is_none(),
            "Person label bucket cleared after truncate"
        );
    }

    // The bound type survives the truncate: the graph is still closed AND a fresh
    // valid Person insert still commits against the still-declared type.
    assert!(
        shared.is_closed(),
        "TRUNCATE keeps the graph closed / bound type intact"
    );
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        m.create_node(
            LabelSet::single(istr("Person")),
            prop("name", Value::String(istr("Carol"))),
        )
        .unwrap();
    }
    txn.commit()
        .expect("Person type still declared after truncate — a valid insert still commits");
    assert_eq!(shared.read().node_count(), 1);
}
