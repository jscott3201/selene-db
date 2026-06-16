use super::*;

#[test]
fn set_property_evaluates_per_row_against_input_row() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n.age = n.age + 12 RETURN n.age AS age");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(column_values(&table, "age"), [Value::Int(42)]);
}

#[test]
fn set_property_to_null_stores_explicit_null() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n.age = NULL RETURN n");

    let (table, _) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert_eq!(
        graph
            .read()
            .node_properties(id)
            .unwrap()
            .get(&db_string("age")),
        Some(&Value::Null)
    );
}

#[test]
fn set_label_on_existing_node_adds_label_idempotently() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n :Person RETURN n");

    let (table, _) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert!(
        graph
            .read()
            .node_labels(id)
            .unwrap()
            .contains(&db_string("Person"))
    );
}

#[test]
fn remove_property_removes_when_present() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) REMOVE n.age RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert_eq!(
        graph
            .read()
            .node_properties(id)
            .unwrap()
            .get(&db_string("age")),
        None
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodePropertyRemoved { id: changed, property }]
            if *changed == id && *property == db_string("age")
    ));
}

#[test]
fn remove_nonexistent_property_is_idempotent() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) REMOVE n.missing RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert!(graph.read().is_node_alive(id));
    assert!(outcome.changes.is_empty());
}

#[test]
fn remove_label_removes_when_present() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) REMOVE n :Person RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert!(
        !graph
            .read()
            .node_labels(id)
            .unwrap()
            .contains(&db_string("Person"))
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeLabelRemoved { id: changed, label }]
            if *changed == id && *label == db_string("Person")
    ));
}

#[test]
fn remove_edge_property_removes_when_present() {
    let graph = graph_with_edge();
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .update_edge(
                EdgeId::new(1),
                selene_core::PropertyDiff::new([(db_string("since"), Value::Int(2026))], [])
                    .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("MATCH ()-[r:REL]->() REMOVE r.since FINISH");

    let (_, outcome) = run_write(&graph, &plan).expect("write executes");

    assert!(
        graph
            .read()
            .edge_properties(EdgeId::new(1))
            .unwrap()
            .get(&db_string("since"))
            .is_none()
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::EdgePropertyRemoved { id, property }]
            if *id == EdgeId::new(1) && *property == db_string("since")
    ));
}

#[test]
fn set_then_remove_emits_dedicated_changes_in_source_order() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n.age = 31 REMOVE n.name FINISH");

    let (_, outcome) = run_write(&graph, &plan).expect("write executes");

    let snapshot = graph.read();
    let properties = snapshot.node_properties(NodeId::new(1)).unwrap();
    assert_eq!(properties.get(&db_string("age")), Some(&Value::Int(31)));
    assert!(properties.get(&db_string("name")).is_none());
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeUpdated { id: updated, .. },
            Change::NodePropertyRemoved {
                id: removed,
                property
            }
        ] if *updated == NodeId::new(1)
            && *removed == NodeId::new(1)
            && *property == db_string("name")
    ));
}
