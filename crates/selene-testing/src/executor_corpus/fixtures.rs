//! Executor corpus graph fixtures.

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, PropertyValueType, Value, intern};
use selene_graph::{
    GraphTypeDef, NodeTypeDef, PropertyTypeDef, SharedGraph, TypedIndexKind, ValidationMode,
};

/// Deterministic graph fixture selector for executor corpus entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutorCorpusFixture {
    /// Open graph with no seed data.
    EmptyOpen,
    /// Closed graph with an empty graph type and no seed data.
    EmptyClosed,
    /// Closed graph with a `Person` type and no seed data.
    EmptyPersonClosed,
    /// Open graph with deterministic `Person` nodes, `KNOWS` edges, and optional indexes.
    StandardSeeded {
        /// Number of `Person` nodes to seed.
        node_count: usize,
        /// Whether to build real runtime property indexes.
        with_indexes: bool,
    },
}

impl ExecutorCorpusFixture {
    /// Build a fresh graph for one corpus entry.
    #[must_use]
    pub fn build(self, graph_id: GraphId) -> SharedGraph {
        match self {
            Self::EmptyOpen => SharedGraph::new(graph_id),
            Self::EmptyClosed => closed(graph_id, empty_graph_type()),
            Self::EmptyPersonClosed => closed(graph_id, person_graph_type()),
            Self::StandardSeeded {
                node_count,
                with_indexes,
            } => standard_seeded(graph_id, node_count, with_indexes),
        }
    }
}

fn closed(graph_id: GraphId, graph_type: GraphTypeDef) -> SharedGraph {
    SharedGraph::builder(graph_id)
        .bound_to(graph_type)
        .expect("fixture graph type is valid")
        .build()
        .expect("fixture graph builds")
}

fn standard_seeded(graph_id: GraphId, node_count: usize, with_indexes: bool) -> SharedGraph {
    let node_count = node_count.max(4);
    let graph = SharedGraph::new(graph_id);
    let person = istr("Person");
    let knows = istr("KNOWS");
    let age = istr("age");
    let email = istr("email");
    let kind = istr("kind");
    let name = istr("name");
    let score = istr("score");
    let tenant = istr("tenant");

    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for idx in 0..node_count {
            mutator
                .create_node(
                    LabelSet::single(person.clone()),
                    PropertyMap::from_pairs([
                        (age.clone(), Value::Int(20 + idx as i64)),
                        (
                            email.clone(),
                            Value::String(istr(&format!("{}@example.com", name_for(idx)))),
                        ),
                        (kind.clone(), Value::String(istr("person"))),
                        (name.clone(), Value::String(istr(name_for(idx)))),
                        (score.clone(), Value::Int((idx + 1) as i64)),
                        (
                            tenant.clone(),
                            Value::String(if idx % 2 == 0 { istr("t1") } else { istr("t2") }),
                        ),
                    ])
                    .expect("fixture properties fit core caps"),
                )
                .expect("fixture node inserts");
        }
        for idx in 1..node_count.min(3) {
            mutator
                .create_edge(
                    knows.clone(),
                    selene_core::NodeId::new(idx as u64),
                    selene_core::NodeId::new(idx as u64 + 1),
                    PropertyMap::from_pairs([(score.clone(), Value::Int(idx as i64))])
                        .expect("fixture edge properties fit core caps"),
                )
                .expect("fixture edge inserts");
        }
        txn.commit().expect("fixture commit succeeds");
    }

    if with_indexes {
        graph
            .create_property_index(person.clone(), age, TypedIndexKind::I64)
            .expect("age index builds");
        graph
            .create_property_index(person.clone(), email, TypedIndexKind::String)
            .expect("email index builds");
        graph
            .create_property_index(person.clone(), kind, TypedIndexKind::String)
            .expect("kind index builds");
        graph
            .create_property_index(person, tenant, TypedIndexKind::String)
            .expect("tenant index builds");
    }
    graph
}

fn empty_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("fixture.executor_empty"),
        node_types: Vec::new(),
        edge_types: Vec::new(),
    }
}

fn person_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("fixture.executor_person"),
        node_types: vec![NodeTypeDef {
            name: istr("Person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![property("name", PropertyValueType::String, false)],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn property(name: &str, value_type: PropertyValueType, required: bool) -> PropertyTypeDef {
    PropertyTypeDef {
        name: istr(name),
        value_type,
        list_element_type: None,
        required,
        default: None,
        immutable: false,
        record_field_types: None,
    }
}

fn name_for(idx: usize) -> &'static str {
    const NAMES: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "h"];
    NAMES[idx % NAMES.len()]
}

fn istr(value: &str) -> IStr {
    intern(value).expect("fixture strings fit interner")
}
