//! BRIEF-37 catalog pipeline executor tests.

mod exec_common;

use selene_core::{Change, GraphId, LabelSet, PropertyMap, SchemaChange, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, CatalogOp, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, GqlStatus, GqlType, PipelineOp, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, RecordType, SourceSpan, TxContext, analyze, execute_pipeline, parse,
    plan,
};
use selene_graph::EdgeTypeDef;
use selene_graph::{GraphError, GraphTypeDef, NodeTypeDef, SharedGraph};

use exec_common::istr;

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    )
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("catalog.test.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn person_graph(id: u64) -> SharedGraph {
    let person = istr("Person");
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("catalog.person.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
            }],
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn closed_graph_with_type(id: u64, graph_type: GraphTypeDef) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap()
}

fn run_write(
    graph: &SharedGraph,
    plan: &ExecutionPlan,
) -> Result<
    (
        BindingTable,
        Result<selene_graph::CommitOutcome, GraphError>,
    ),
    ExecutorError,
> {
    let snapshot = graph.read();
    let mut txn = graph.begin_write();
    let result = {
        let mut ctx = TxContext::write(
            snapshot,
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            &mut txn,
            graph.index_providers(),
        );
        execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
    };
    match result {
        Ok(table) => Ok((table, txn.commit())),
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

fn catalog_op(source: &str) -> PipelineOp {
    planned(source).pipeline.remove(0)
}

fn create_node_type_op(label: &str) -> PipelineOp {
    catalog_op(&format!("CREATE NODE TYPE :{label} ()"))
}

#[test]
fn create_node_type_creates_type_and_preserves_input_row() {
    let graph = empty_closed_graph(3700);
    let plan = planned("CREATE NODE TYPE :Person (name :: STRING NOT NULL)");

    let (table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    let outcome = outcome.expect("commit succeeds");

    assert_eq!(table.row_count(), 1);
    let graph_type = graph.graph_type().unwrap();
    assert_eq!(graph_type.node_types[0].name.as_str(), "Person");
    assert_eq!(graph_type.node_types[0].properties[0].name.as_str(), "name");
    assert!(graph_type.node_types[0].properties[0].required);
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAdded { label, .. },
            ..
        }] if label.as_str() == "Person"
    ));
}

#[test]
fn create_node_type_indexed_property_creates_property_index() {
    let graph = empty_closed_graph(3717);
    let plan = planned("CREATE NODE TYPE :Sensor (serial :: STRING INDEXED)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    let outcome = outcome.expect("commit succeeds");

    let sensor = istr("Sensor");
    let serial = istr("serial");
    assert!(graph.read().property_index_for(&sensor, &serial).is_some());
    assert_eq!(graph.read().property_index_count(), 1);
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::SchemaChanged {
                change: SchemaChange::NodeTypeAdded { label, .. },
                ..
            },
            Change::SchemaChanged {
                change: SchemaChange::PropertyIndexCreated { label: index_label, property, .. },
                ..
            }
        ] if label.as_str() == "Sensor"
            && index_label.as_str() == "Sensor"
            && property.as_str() == "serial"
    ));
}

#[test]
fn create_node_type_unindexed_property_does_not_create_property_index() {
    let graph = empty_closed_graph(3718);
    let plan = planned("CREATE NODE TYPE :Sensor (serial :: STRING)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    assert_eq!(graph.read().property_index_count(), 0);
}

#[test]
fn create_node_type_indexed_unsupported_type_reports_feature_not_supported() {
    let graph = empty_closed_graph(3719);
    let plan = planned("CREATE NODE TYPE :Sensor (active :: BOOLEAN INDEXED)");

    let err = run_write(&graph, &plan).expect_err("BOOLEAN inline index unsupported");

    assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
}

#[test]
fn show_node_types_on_open_graph_returns_empty_schemaful_table() {
    let graph = SharedGraph::new(GraphId::new(3701));
    let plan = planned("SHOW NODE TYPES");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect("show executes");

    assert_eq!(table.row_count(), 0);
    assert_eq!(table.schema().columns[0].name.unwrap().as_str(), "label");
    assert_eq!(
        table.schema().columns[1].name.unwrap().as_str(),
        "definition"
    );
}

#[test]
fn show_node_types_after_create_in_same_statement_includes_new_type() {
    let graph = empty_closed_graph(3702);
    let mut plan = planned("SHOW NODE TYPES");
    plan.pipeline.insert(
        0,
        catalog_op("CREATE NODE TYPE :Person (name :: STRING NOT NULL)"),
    );

    let (table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values()[0], Value::String(istr("Person")));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(istr("CREATE NODE TYPE :Person (name :: STRING NOT NULL)"))
    );
}

#[test]
fn show_node_types_renders_key_labels_not_internal_name() {
    let graph = closed_graph_with_type(
        3717,
        GraphTypeDef {
            name: istr("catalog.asymmetric.graph"),
            node_types: vec![NodeTypeDef {
                name: istr("types.person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: Vec::new(),
            }],
            edge_types: Vec::new(),
        },
    );
    let plan = planned("SHOW NODE TYPES");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect("show executes");

    assert_eq!(table.rows()[0].values()[0], Value::String(istr("Person")));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(istr("CREATE NODE TYPE :Person ()"))
    );
}

#[test]
fn create_edge_type_resolves_endpoints_created_earlier_in_same_statement() {
    let graph = empty_closed_graph(3703);
    let mut plan = planned("CREATE EDGE TYPE :WORKS_AT (FROM :Person TO :Company)");
    plan.pipeline.insert(0, create_node_type_op("Company"));
    plan.pipeline.insert(0, create_node_type_op("Person"));

    let (_, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let graph_type = graph.graph_type().unwrap();
    assert_eq!(graph_type.edge_types[0].name.as_str(), "WORKS_AT");
    assert_eq!(graph_type.edge_types[0].source_node_type, 0);
    assert_eq!(graph_type.edge_types[0].target_node_type, 1);
}

#[test]
fn show_edge_types_renders_round_trippable_definition() {
    let graph = empty_closed_graph(3704);
    let mut plan = planned("SHOW EDGE TYPES");
    plan.pipeline.insert(
        0,
        catalog_op("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person, since :: DATE)"),
    );
    plan.pipeline.insert(0, create_node_type_op("Person"));

    let (table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let Value::String(definition) = table.rows()[0].values()[1] else {
        panic!("definition is string");
    };
    assert_eq!(
        definition.as_str(),
        "CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person, since :: DATE)"
    );
    parse(definition.as_str()).expect("definition round-trips through parser");
}

#[test]
fn show_edge_types_renders_label_not_internal_name() {
    let person = istr("Person");
    let knows = istr("KNOWS");
    let graph = closed_graph_with_type(
        3718,
        GraphTypeDef {
            name: istr("catalog.edge.asymmetric.graph"),
            node_types: vec![NodeTypeDef {
                name: istr("types.person"),
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
            }],
            edge_types: vec![EdgeTypeDef {
                name: istr("types.knows"),
                label: knows,
                source_node_type: 0,
                target_node_type: 0,
                properties: Vec::new(),
            }],
        },
    );
    let plan = planned("SHOW EDGE TYPES");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect("show executes");

    assert_eq!(table.rows()[0].values()[0], Value::String(knows));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(istr("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person)"))
    );
}

#[test]
fn show_edge_types_renders_multi_label_endpoint_labels() {
    let knows = istr("KNOWS");
    let labels = LabelSet::from_iter([istr("Person"), istr("Active")]);
    let graph = closed_graph_with_type(
        3719,
        GraphTypeDef {
            name: istr("catalog.edge.multilabel.graph"),
            node_types: vec![NodeTypeDef {
                name: istr("types.active_person"),
                key_labels: labels,
                properties: Vec::new(),
            }],
            edge_types: vec![EdgeTypeDef {
                name: istr("types.knows"),
                label: knows,
                source_node_type: 0,
                target_node_type: 0,
                properties: Vec::new(),
            }],
        },
    );
    let plan = planned("SHOW EDGE TYPES");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect("show executes");

    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(istr(
            "CREATE EDGE TYPE :KNOWS (FROM :Person:Active TO :Person:Active)"
        ))
    );
}

#[test]
fn create_node_type_then_insert_same_statement_validates_against_new_type() {
    let graph = empty_closed_graph(3705);
    let mut plan = planned("INSERT (n:Person {name: 'Ada'}) RETURN n");
    plan.pipeline.insert(
        0,
        catalog_op("CREATE NODE TYPE :Person (name :: STRING NOT NULL)"),
    );

    let (table, outcome) = run_write(&graph, &plan).expect("catalog plus mutation executes");
    outcome.expect("commit succeeds");

    assert_eq!(table.row_count(), 1);
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn open_graph_create_node_type_returns_data_exception() {
    let graph = SharedGraph::new(GraphId::new(3706));
    let plan = planned("CREATE NODE TYPE :Person ()");

    let err = run_write(&graph, &plan).expect_err("open graph catalog DDL fails");

    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("open graph (GG01) does not support catalog type DDL")
    ));
}

#[test]
fn catalog_op_without_write_txn_returns_invalid_transaction_state() {
    let graph = empty_closed_graph(3707);
    let plan = planned("CREATE NODE TYPE :Person ()");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
        .expect_err("catalog DDL needs write tx");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "catalog op invoked without write transaction",
            ..
        }
    ));
}

#[test]
fn create_edge_type_without_write_txn_returns_invalid_transaction_state() {
    let graph = empty_closed_graph(3720);
    let plan = planned("CREATE EDGE TYPE :KNOWS (FROM :Missing TO :Missing)");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
        .expect_err("catalog DDL needs write tx before endpoint lookup");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "catalog op invoked without write transaction",
            ..
        }
    ));
}

#[test]
fn create_edge_type_unknown_endpoint_returns_data_exception() {
    let graph = empty_closed_graph(3708);
    let plan = planned("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person)");

    let err = run_write(&graph, &plan).expect_err("unknown endpoint fails");

    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("unknown node type label Person")
    ));
}

#[test]
fn create_edge_type_multi_label_endpoint_returns_implementation_defined() {
    let graph = person_graph(3709);
    let mut plan = planned("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person)");
    let PipelineOp::Catalog(CatalogOp::CreateEdgeType { endpoints, .. }) = &mut plan.pipeline[0]
    else {
        panic!("expected create edge type");
    };
    endpoints
        .as_mut()
        .unwrap()
        .from_labels
        .push(istr("Employee"));

    let err = run_write(&graph, &plan).expect_err("multi-label endpoint deferred");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "multi-label edge endpoint not supported (Phase A: single label per endpoint)",
        }
    ));
}

#[test]
fn create_edge_type_without_endpoints_returns_implementation_defined() {
    let graph = person_graph(3710);
    let mut plan = planned("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person)");
    let PipelineOp::Catalog(CatalogOp::CreateEdgeType { endpoints, .. }) = &mut plan.pipeline[0]
    else {
        panic!("expected create edge type");
    };
    *endpoints = None;

    let err = run_write(&graph, &plan).expect_err("missing endpoints deferred");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "create edge type without endpoints requires open graph (GG01)",
        }
    ));
}

#[test]
fn drop_node_type_with_existing_nodes_errors_at_commit() {
    let graph = person_graph(3711);
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("DROP NODE TYPE :Person");

    let (_, outcome) = run_write(&graph, &plan).expect("drop op itself executes");
    assert!(matches!(
        outcome.expect_err("commit rejects invalid post-schema state"),
        GraphError::TypeViolation(_)
    ));
}

#[test]
fn drop_nonexistent_node_type_returns_data_exception() {
    let graph = empty_closed_graph(3712);
    let plan = planned("DROP NODE TYPE :Missing");

    let err = run_write(&graph, &plan).expect_err("drop missing fails");

    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("node type Missing does not exist")
    ));
}

#[test]
fn phase_a_flags_and_constraints_are_deferred() {
    let graph = empty_closed_graph(3714);
    let mut cases = Vec::new();

    let mut or_replace = planned("CREATE NODE TYPE :Person ()");
    if let PipelineOp::Catalog(CatalogOp::CreateNodeType { or_replace, .. }) =
        &mut or_replace.pipeline[0]
    {
        *or_replace = true;
    }
    cases.push(or_replace);

    let mut if_not_exists = planned("CREATE NODE TYPE :Person ()");
    if let PipelineOp::Catalog(CatalogOp::CreateNodeType { if_not_exists, .. }) =
        &mut if_not_exists.pipeline[0]
    {
        *if_not_exists = true;
    }
    cases.push(if_not_exists);

    let mut extends = planned("CREATE NODE TYPE :Person ()");
    if let PipelineOp::Catalog(CatalogOp::CreateNodeType { extends, .. }) = &mut extends.pipeline[0]
    {
        *extends = Some(istr("Entity"));
    }
    cases.push(extends);

    cases.push(planned("CREATE NODE TYPE :Sensor (v :: STRING UNIQUE)"));
    cases.push(planned("DROP NODE TYPE IF EXISTS :Missing"));

    for plan in cases {
        let err = run_write(&graph, &plan).expect_err("phase A surface is deferred");
        assert!(matches!(err, ExecutorError::ImplementationDefined { .. }));
    }
}

#[test]
fn record_list_and_nothing_property_types_are_deferred() {
    let graph = empty_closed_graph(3715);
    for gql_type in [
        GqlType::Record(RecordType::Open),
        GqlType::List(Box::new(GqlType::Integer)),
        GqlType::Nothing,
    ] {
        let mut plan = planned("CREATE NODE TYPE :Person ()");
        let PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. }) =
            &mut plan.pipeline[0]
        else {
            panic!("expected create node type");
        };
        properties.push(PlannedTypePropertyDef {
            name: istr("payload"),
            gql_type,
            constraints: Vec::new(),
            span: SourceSpan::new(0, 1),
        });

        let err = run_write(&graph, &plan).expect_err("type is deferred");

        assert!(matches!(
            err,
            ExecutorError::ImplementationDefined {
                detail: "type property GQL type not supported as property value type (Phase A)",
            }
        ));
    }
}

#[test]
fn default_property_constraint_is_deferred_from_hand_built_ir() {
    let graph = empty_closed_graph(3716);
    let mut plan = planned("CREATE NODE TYPE :Person ()");
    let PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. }) = &mut plan.pipeline[0]
    else {
        panic!("expected create node type");
    };
    let project = planned("RETURN 1 AS x")
        .pipeline
        .into_iter()
        .find_map(|op| match op {
            PipelineOp::Project(mut items) => items.pop(),
            _ => None,
        })
        .expect("project expr");
    properties.push(PlannedTypePropertyDef {
        name: istr("age"),
        gql_type: GqlType::Integer,
        constraints: vec![PlannedTypePropertyConstraint::Default(
            project,
            SourceSpan::new(0, 1),
        )],
        span: SourceSpan::new(0, 1),
    });

    let err = run_write(&graph, &plan).expect_err("default constraint is deferred");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "type property constraint not implemented (Phase A: NOT NULL only)",
        }
    ));
}
