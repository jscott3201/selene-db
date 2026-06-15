use super::*;

#[test]
fn match_after_insert_in_same_statement_sees_inserted_node() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person {name: 'Zed'}) RETURN n.name AS name");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(
        column_values(&table, "name"),
        [Value::String(db_string("Zed"))]
    );
}

#[test]
fn mutation_without_write_txn_returns_invalid_transaction_state() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person) RETURN n");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect_err("write errors");

    assert!(matches!(err, ExecutorError::InvalidTransactionState { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::READ_ONLY_TRANSACTION_VIOLATION);
}

#[test]
fn mutator_error_surfaces_as_graph_mutation_executor_error() {
    let graph = empty_graph();
    let plan = planned("MATCH (n) SET n.age = 1 FINISH");
    let schema = BindingTableSchema {
        columns: vec![BindingTableColumn {
            name: Some(db_string("n")),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::NodeRef),
        }],
    };
    let table = BindingTable::new(
        schema,
        vec![Binding::new([Value::NodeRef(NodeId::new(999))])],
    );
    let snapshot = graph.read();
    let mut txn = graph.begin_write();
    let mut ctx = TxContext::write(
        snapshot,
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        &mut txn,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, table, &mut ctx).expect_err("write errors");

    assert!(matches!(err, ExecutorError::GraphMutation { .. }));
}

#[test]
fn mutation_routes_through_mutator_only() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person) FINISH");

    let (_, outcome) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(outcome.changes.len(), 1);
}

#[test]
fn mutation_over_empty_input_table_produces_no_changes() {
    let graph = empty_graph();
    let plan = planned("MATCH (n:Missing) INSERT (m:New) RETURN m");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(table.row_count(), 0);
    assert!(outcome.changes.is_empty());
}

#[test]
fn mutation_aborts_op_on_first_row_error_no_partial_rollback() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n) SET n.age = 1 FINISH");
    let alice = graph
        .read()
        .node_store
        .alive
        .iter()
        .next()
        .map(|row| NodeId::new(u64::from(row) + 1))
        .expect("fixture has a node");
    let schema = BindingTableSchema {
        columns: vec![BindingTableColumn {
            name: Some(db_string("n")),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::NodeRef),
        }],
    };
    let table = BindingTable::new(
        schema,
        vec![
            Binding::new([Value::NodeRef(alice)]),
            Binding::new([Value::NodeRef(NodeId::new(999))]),
        ],
    );
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
        execute_pipeline(&plan.pipeline, table, &mut ctx)
    };

    assert!(matches!(result, Err(ExecutorError::GraphMutation { .. })));
    assert_eq!(
        txn.read()
            .node_properties(alice)
            .unwrap()
            .get(&db_string("age")),
        Some(&Value::Int(1))
    );
    txn.rollback();
}
