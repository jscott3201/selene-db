//! End-to-end coverage for native `selene.*` text-search built-ins.

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
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
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn execute_ok(session: &mut Session<'_>, source: &str, registry: &dyn ProcedureRegistry) {
    session
        .execute_source(source, registry)
        .expect("statement executes");
}

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(value)) => *value,
            other => panic!("expected node ref in {name}, got {other:?}"),
        })
        .collect()
}

fn float_column(table: &BindingTable, name: &str) -> Vec<f64> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Float(value)) => *value,
            other => panic!("expected float in {name}, got {other:?}"),
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

#[test]
fn text_search_nodes_ranks_string_properties() {
    let graph = graph(431_101);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("TextDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("graph memory graph retrieval"))),
            )
            .expect("text node inserts");
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("vector retrieval retrieval"))),
            )
            .expect("text node inserts");
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("graph search"))),
            )
            .expect("text node inserts");
        mutator
            .create_node(LabelSet::single(doc.clone()), props(&body, Value::Int(1)))
            .expect("non-string node inserts");
        txn.commit().expect("seed commits");
    }

    let table = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', 'graph retrieval', 3) \
         YIELD node_id, score",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(1), NodeId::new(2), NodeId::new(3)]
    );
    let scores = float_column(&table, "score");
    assert!(scores[0] > scores[1]);
    assert!(scores[1] > scores[2]);
}

#[test]
fn create_text_index_commits_and_stats_reports_bm25_state() {
    let graph = graph(431_104);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("TextDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("graph memory graph retrieval"))),
            )
            .expect("text node inserts");
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, Value::String(istr("vector retrieval"))),
            )
            .expect("text node inserts");
        mutator
            .create_node(LabelSet::single(doc), props(&body, Value::Int(7)))
            .expect("non-string node inserts");
        txn.commit().expect("seed commits");
    }

    execute_ok(
        &mut session,
        "CALL selene.create_text_index('TextDoc', 'body', 'body_idx')",
        &registry,
    );

    assert_eq!(graph.read().text_index_count(), 1);
    assert_eq!(
        graph
            .read()
            .text_index_for(&istr("TextDoc"), &body)
            .unwrap()
            .search("graph", 10)
            .into_iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![NodeId::new(1)]
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.text_index_stats() \
         YIELD name, label, property, indexed_rows, documents, distinct_terms, postings, \
               total_document_len, document_term_bytes, estimated_index_bytes",
        &registry,
    );

    assert_eq!(string_column(&table, "name"), vec!["body_idx".to_owned()]);
    assert_eq!(string_column(&table, "label"), vec!["TextDoc".to_owned()]);
    assert_eq!(string_column(&table, "property"), vec!["body".to_owned()]);
    assert_eq!(uint_column(&table, "indexed_rows"), vec![2]);
    assert_eq!(uint_column(&table, "documents"), vec![2]);
    assert_eq!(uint_column(&table, "distinct_terms"), vec![4]);
    assert_eq!(uint_column(&table, "postings"), vec![5]);
    assert_eq!(uint_column(&table, "total_document_len"), vec![6]);
    assert!(uint_column(&table, "document_term_bytes")[0] > 0);
    assert!(uint_column(&table, "estimated_index_bytes")[0] > 0);
}

#[test]
fn create_text_index_rejects_duplicate_registration() {
    let graph = graph(431_105);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    execute_ok(
        &mut session,
        "CALL selene.create_text_index('TextDoc', 'body')",
        &registry,
    );
    let err = session
        .execute_source(
            "CALL selene.create_text_index('TextDoc', 'body')",
            &registry,
        )
        .expect_err("duplicate text index should fail");

    let ExecutorError::Procedure {
        source: ProcedureError::InvalidArgument { detail },
        ..
    } = err
    else {
        panic!("expected invalid procedure argument, got {err:?}");
    };
    assert!(detail.contains("already exists"));
}

#[test]
fn drop_text_index_removes_registration() {
    let graph = graph(431_106);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    execute_ok(
        &mut session,
        "CALL selene.create_text_index('TextDoc', 'body')",
        &registry,
    );
    assert_eq!(graph.read().text_index_count(), 1);

    execute_ok(
        &mut session,
        "CALL selene.drop_text_index('TextDoc', 'body')",
        &registry,
    );

    assert_eq!(graph.read().text_index_count(), 0);
    let table = execute_rows(
        &mut session,
        "CALL selene.text_index_stats() YIELD name",
        &registry,
    );
    assert_eq!(table.row_count(), 0);
}

#[test]
fn text_search_nodes_rejects_negative_k() {
    let graph = graph(431_102);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.text_search_nodes('TextDoc', 'body', 'graph', -1)",
            &registry,
        )
        .expect_err("negative k should fail");

    let ExecutorError::Procedure {
        source: ProcedureError::InvalidArgument { detail },
        ..
    } = err
    else {
        panic!("expected invalid procedure argument, got {err:?}");
    };
    assert!(detail.contains("k must be a non-negative INTEGER"));
}

#[test]
fn text_search_nodes_empty_token_query_returns_no_rows() {
    let graph = graph(431_103);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("TextDoc");
    let body = istr("body");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc),
                props(&body, Value::String(istr("graph memory"))),
            )
            .expect("text node inserts");
        txn.commit().expect("seed commits");
    }

    let table = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', '!!!', 3) \
         YIELD node_id, score",
        &registry,
    );

    assert_eq!(table.row_count(), 0);

    let table = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', '', 3) \
         YIELD node_id, score",
        &registry,
    );

    assert_eq!(table.row_count(), 0);
}
