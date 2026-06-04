//! End-to-end coverage for IVF vector-index maintenance built-ins.

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, VectorValue, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &BuiltinProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn string_column(table: &BindingTable, name: &str) -> Vec<String> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
        })
        .collect()
}

fn uint_column(table: &BindingTable, name: &str) -> Vec<u64> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Uint(value)) => *value,
            other => panic!("expected uint in {name}, got {other:?}"),
        })
        .collect()
}

fn insert_vectors(graph: &SharedGraph, label: &IStr, property: &IStr, count: usize, offset: f32) {
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    for value in 0..count {
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                props(
                    property,
                    Value::Vector(vector(&[offset + value as f32, 0.0])),
                ),
            )
            .expect("vector insert succeeds");
    }
    txn.commit().expect("vector insert commits");
}

#[test]
fn rebuild_recommended_vector_indexes_accepts_optional_cap() {
    let graph = SharedGraph::new(GraphId::new(330_157));
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let high = istr("HighVectorDoc");
    let low = istr("LowVectorDoc");
    let cold = istr("ColdVectorDoc");
    let embedding = istr("embedding");
    for (label, offset) in [(&high, 0.0), (&low, 10_000.0), (&cold, 20_000.0)] {
        insert_vectors(&graph, label, &embedding, 100, offset);
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('HighVectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("high ivf index creates");
    session
        .execute_source(
            "CALL selene.create_vector_index('LowVectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("low ivf index creates");
    session
        .execute_source(
            "CALL selene.create_vector_index('ColdVectorDoc', 'embedding', 2, 'ivf')",
            &registry,
        )
        .expect("cold ivf index creates");

    insert_vectors(&graph, &high, &embedding, 200, 30_000.0);
    insert_vectors(&graph, &low, &embedding, 100, 40_000.0);
    insert_vectors(&graph, &cold, &embedding, 1, 50_000.0);

    let first = execute_rows(
        &mut session,
        "CALL selene.rebuild_recommended_vector_indexes(1) \
         YIELD label, before_ivf_pending_retrain_basis_points",
        &registry,
    );
    assert_eq!(first.row_count(), 1);
    assert_eq!(string_column(&first, "label"), vec!["HighVectorDoc"]);
    assert_eq!(
        uint_column(&first, "before_ivf_pending_retrain_basis_points"),
        vec![6_666]
    );

    let second = execute_rows(
        &mut session,
        "CALL selene.rebuild_recommended_vector_indexes(NULL) \
         YIELD label, before_ivf_pending_retrain_basis_points",
        &registry,
    );
    assert_eq!(second.row_count(), 1);
    assert_eq!(string_column(&second, "label"), vec!["LowVectorDoc"]);
    assert_eq!(
        uint_column(&second, "before_ivf_pending_retrain_basis_points"),
        vec![5_000]
    );

    let err = session
        .execute_source(
            "CALL selene.rebuild_recommended_vector_indexes(0)",
            &registry,
        )
        .expect_err("zero max_indexes is rejected");
    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("max_indexes must be NULL or a positive INTEGER")
    ));
}
