//! BRIEF-120 procedure metadata coverage.

use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, GqlType, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn full_registry() -> BuiltinProcedureRegistry {
    BuiltinProcedureRegistry::new()
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

fn column_strings(table: &BindingTable, name: &str) -> Vec<String> {
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

fn semver_like(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && major.parse::<u64>().is_ok()
        && minor.parse::<u64>().is_ok()
        && patch.parse::<u64>().is_ok()
}

#[test]
fn default_registry_exposes_non_empty_metadata_for_all_37_procedures() {
    let registry = full_registry();
    let procedures = registry.iter_handles().collect::<Vec<_>>();

    assert_eq!(procedures.len(), 37);
    for (name, metadata) in procedures {
        let rendered = name
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(".");
        assert!(
            !metadata.description.is_empty(),
            "{rendered} missing procedure description"
        );
        assert!(
            semver_like(metadata.signature.since_version),
            "{rendered} has invalid since_version {}",
            metadata.signature.since_version
        );
        for parameter in &metadata.signature.parameters {
            assert!(
                !parameter.description.is_empty(),
                "{rendered}.{} missing parameter description",
                parameter.name
            );
        }
        for column in &metadata.output_schema.columns {
            assert!(
                !column.description.is_empty(),
                "{rendered}.{} missing output description",
                column.name
            );
        }
    }
}

#[test]
fn show_procedures_exposes_six_columns_and_zero_arg_description() {
    let graph = SharedGraph::new(GraphId::new(120_001));
    let registry = full_registry();
    let mut session = Session::new(&graph);
    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let columns = table
        .schema()
        .columns
        .iter()
        .map(|column| {
            column
                .name
                .as_ref()
                .expect("SHOW PROCEDURES columns are named")
                .as_str()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        columns,
        vec![
            "name",
            "tier",
            "mutability",
            "signature",
            "description",
            "since_version",
        ]
    );
    assert_eq!(table.row_count(), 37);

    let names = column_strings(&table, "name");
    let descriptions = column_strings(&table, "description");
    let health = names
        .iter()
        .position(|name| name == "selene.health")
        .expect("selene.health is registered");
    assert_eq!(descriptions[health], "Report basic graph health counters.");
    assert!(names.iter().any(|name| name == "selene.vector_index_stats"));
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_nodes_batch")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_neighbors")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_neighbors_batch")
    );
    let rebuild = names
        .iter()
        .position(|name| name == "selene.rebuild_vector_indexes")
        .expect("selene.rebuild_vector_indexes is registered");
    assert_eq!(column_strings(&table, "tier")[rebuild], "maintenance");
    assert_eq!(
        column_strings(&table, "mutability")[rebuild],
        "maintenance_write"
    );
    let rebuild_recommended = names
        .iter()
        .position(|name| name == "selene.rebuild_recommended_vector_indexes")
        .expect("selene.rebuild_recommended_vector_indexes is registered");
    assert_eq!(
        column_strings(&table, "tier")[rebuild_recommended],
        "maintenance"
    );
    assert_eq!(
        column_strings(&table, "mutability")[rebuild_recommended],
        "maintenance_write"
    );
}

#[test]
fn vector_score_nodes_batch_metadata_has_nested_node_candidates() {
    let registry = full_registry();
    let name = [istr("selene"), istr("vector_score_nodes_batch")];
    let metadata = registry
        .lookup(&name)
        .expect("vector_score_nodes_batch resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 5);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "queries");
    assert_eq!(parameters[1].ty, GqlType::List(Box::new(GqlType::Vector)));
    assert_eq!(parameters[2].name.as_str(), "nodes");
    assert_eq!(
        parameters[2].ty,
        GqlType::List(Box::new(GqlType::List(Box::new(GqlType::NodeRef))))
    );
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, GqlType::Integer);
    assert_eq!(parameters[4].name.as_str(), "metric");
    assert_eq!(parameters[4].ty, GqlType::String);
    assert_eq!(parameters[4].default_doc, Some("squared_euclidean"));
    assert!(parameters[4].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "distance");
    assert_eq!(columns[2].ty, GqlType::Float64);
}
