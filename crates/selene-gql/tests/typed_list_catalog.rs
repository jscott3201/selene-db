//! Catalog DDL coverage for typed `LIST<T>` property declarations.

mod exec_common;

use selene_core::{GraphId, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, TxContext, analyze, execute_pipeline, parse, plan,
};
use selene_graph::{GraphError, GraphTypeDef, PropertyElementType, SharedGraph};

use exec_common::db_string;

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
            name: db_string("catalog.typed.list.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
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

#[test]
fn create_node_type_accepts_and_renders_typed_list_property() {
    let graph = empty_closed_graph(13_100);
    let plan = planned(
        "CREATE NODE TYPE :Person \
         (tags :: LIST<STRING>, matrix :: LIST<LIST<INTEGER>>)",
    );

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let graph_type = graph.graph_type().unwrap();
    let tags = &graph_type.node_types[0].properties[0];
    assert_eq!(tags.value_type, selene_core::PropertyValueType::List);
    assert_eq!(
        tags.list_element_type,
        Some(PropertyElementType::Scalar(
            selene_core::PropertyValueType::String
        ))
    );
    let matrix = &graph_type.node_types[0].properties[1];
    assert_eq!(
        matrix.list_element_type,
        Some(PropertyElementType::List(Box::new(
            PropertyElementType::Scalar(selene_core::PropertyValueType::Int)
        )))
    );

    let show = planned("SHOW NODE TYPES");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &show.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );
    let table = execute_pipeline(&show.pipeline, seed_table(), &mut ctx).expect("show executes");
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Person (tags :: LIST<STRING>, matrix :: LIST<LIST<INTEGER>>)"
        ))
    );
}
