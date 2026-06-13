use selene_core::{LabelDiff, PropertyDiff};

use super::*;

#[test]
fn validate_change_accepts_applied_node_created() {
    let graph = valid_graph();
    validate_change(
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(db_string("Person")),
            properties: prop("name", Value::String(db_string("Alice"))),
        },
        &graph,
        &graph_type(),
    )
    .unwrap();
}

#[test]
fn validate_change_skips_incident_edges_for_property_only_node_update() {
    let mut graph = valid_graph();
    graph
        .node_store
        .labels
        .set(0, LabelSet::single(db_string("Company")));

    validate_change(
        &Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([], []).unwrap(),
            properties_diff: PropertyDiff::new(
                [(db_string("name"), Value::String(db_string("Alicia")))],
                [],
            )
            .unwrap(),
        },
        &graph,
        &graph_type(),
    )
    .unwrap();
}
