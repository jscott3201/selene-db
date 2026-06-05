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
fn default_registry_exposes_non_empty_metadata_for_all_52_procedures() {
    let registry = full_registry();
    let procedures = registry.iter_handles().collect::<Vec<_>>();

    assert_eq!(procedures.len(), 52);
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
    assert_eq!(table.row_count(), 52);

    let names = column_strings(&table, "name");
    let descriptions = column_strings(&table, "description");
    let health = names
        .iter()
        .position(|name| name == "selene.health")
        .expect("selene.health is registered");
    assert_eq!(descriptions[health], "Report basic graph health counters.");
    assert!(names.iter().any(|name| name == "selene.vector_index_stats"));
    assert!(names.iter().any(|name| name == "selene.text_index_stats"));
    assert!(names.iter().any(|name| name == "selene.create_text_index"));
    assert!(names.iter().any(|name| name == "selene.drop_text_index"));
    assert!(names.iter().any(|name| name == "selene.text_search_nodes"));
    assert!(names.iter().any(|name| name == "selene.text_score_nodes"));
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_search_expanded_candidates_ann")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_search_candidate_state_expanded_ann")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_search_expanded_candidates_ann_batch")
    );
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
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_expanded_candidates")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_expanded_candidates_batch")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_candidate_state")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_candidate_state_nodes")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_candidate_state_expanded")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_score_candidate_state_expanded_batch")
    );
    assert!(
        names
            .iter()
            .any(|name| name == "selene.vector_candidate_states")
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
fn text_score_nodes_metadata_has_node_candidates() {
    let registry = full_registry();
    let name = [istr("selene"), istr("text_score_nodes")];
    let metadata = registry.lookup(&name).expect("text_score_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 5);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "nodes");
    assert_eq!(parameters[3].ty, GqlType::List(Box::new(GqlType::NodeRef)));
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, GqlType::Integer);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "score");
    assert_eq!(columns[1].ty, GqlType::Float64);
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

#[test]
fn vector_candidate_states_metadata_has_descriptor_columns() {
    let registry = full_registry();
    let name = [istr("selene"), istr("vector_candidate_states")];
    let metadata = registry
        .lookup(&name)
        .expect("vector_candidate_states resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 0);
    assert_eq!(arity.maximum, 0);
    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 8);
    assert_eq!(columns[0].name.as_str(), "state_name");
    assert_eq!(columns[0].ty, GqlType::String);
    assert_eq!(columns[1].name.as_str(), "generation");
    assert_eq!(columns[1].ty, GqlType::Uint64);
    assert_eq!(columns[2].name.as_str(), "candidate_count");
    assert_eq!(columns[2].ty, GqlType::Uint64);
    assert_eq!(columns[3].name.as_str(), "required_label");
    assert_eq!(columns[3].ty, GqlType::String);
    assert_eq!(columns[4].name.as_str(), "require_outgoing");
    assert_eq!(columns[4].ty, GqlType::List(Box::new(GqlType::String)));
    assert_eq!(columns[5].name.as_str(), "require_incoming");
    assert_eq!(columns[5].ty, GqlType::List(Box::new(GqlType::String)));
    assert_eq!(columns[6].name.as_str(), "exclude_outgoing");
    assert_eq!(columns[6].ty, GqlType::List(Box::new(GqlType::String)));
    assert_eq!(columns[7].name.as_str(), "exclude_incoming");
    assert_eq!(columns[7].ty, GqlType::List(Box::new(GqlType::String)));
}

#[test]
fn vector_score_candidate_state_nodes_metadata_has_composition_args() {
    let registry = full_registry();
    let name = [istr("selene"), istr("vector_score_candidate_state_nodes")];
    let metadata = registry
        .lookup(&name)
        .expect("vector_score_candidate_state_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 7);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "state_name");
    assert_eq!(parameters[2].ty, GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "nodes");
    assert_eq!(parameters[3].ty, GqlType::List(Box::new(GqlType::NodeRef)));
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "operation");
    assert_eq!(parameters[5].ty, GqlType::String);
    assert_eq!(parameters[5].default_doc, Some("intersection"));
    assert!(parameters[5].default.is_some());
    assert_eq!(parameters[6].name.as_str(), "metric");
    assert_eq!(parameters[6].ty, GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("squared_euclidean"));
    assert!(parameters[6].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, GqlType::Float64);
}

#[test]
fn vector_score_candidate_state_expanded_metadata_has_expansion_and_composition_args() {
    let registry = full_registry();
    let name = [
        istr("selene"),
        istr("vector_score_candidate_state_expanded"),
    ];
    let metadata = registry
        .lookup(&name)
        .expect("vector_score_candidate_state_expanded resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 6);
    assert_eq!(arity.maximum, 9);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "query");
    assert_eq!(parameters[1].ty, GqlType::Vector);
    assert_eq!(parameters[2].name.as_str(), "state_name");
    assert_eq!(parameters[2].ty, GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "roots");
    assert_eq!(parameters[3].ty, GqlType::List(Box::new(GqlType::NodeRef)));
    assert_eq!(parameters[4].name.as_str(), "edge_label");
    assert_eq!(parameters[4].ty, GqlType::String);
    assert_eq!(parameters[5].name.as_str(), "k");
    assert_eq!(parameters[5].ty, GqlType::Integer);
    assert_eq!(parameters[6].name.as_str(), "operation");
    assert_eq!(parameters[6].ty, GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("intersection"));
    assert!(parameters[6].default.is_some());
    assert_eq!(parameters[7].name.as_str(), "direction");
    assert_eq!(parameters[7].ty, GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("outgoing"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "metric");
    assert_eq!(parameters[8].ty, GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("squared_euclidean"));
    assert!(parameters[8].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, GqlType::Float64);
}

#[test]
fn vector_score_candidate_state_expanded_batch_metadata_has_batch_args() {
    let registry = full_registry();
    let name = [
        istr("selene"),
        istr("vector_score_candidate_state_expanded_batch"),
    ];
    let metadata = registry
        .lookup(&name)
        .expect("vector_score_candidate_state_expanded_batch resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 6);
    assert_eq!(arity.maximum, 9);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "property");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "queries");
    assert_eq!(parameters[1].ty, GqlType::List(Box::new(GqlType::Vector)));
    assert_eq!(parameters[2].name.as_str(), "state_name");
    assert_eq!(parameters[2].ty, GqlType::String);
    assert_eq!(parameters[3].name.as_str(), "roots");
    assert_eq!(
        parameters[3].ty,
        GqlType::List(Box::new(GqlType::List(Box::new(GqlType::NodeRef))))
    );
    assert_eq!(parameters[4].name.as_str(), "edge_label");
    assert_eq!(parameters[4].ty, GqlType::String);
    assert_eq!(parameters[5].name.as_str(), "k");
    assert_eq!(parameters[5].ty, GqlType::Integer);
    assert_eq!(parameters[6].name.as_str(), "operation");
    assert_eq!(parameters[6].ty, GqlType::String);
    assert_eq!(parameters[6].default_doc, Some("intersection"));
    assert!(parameters[6].default.is_some());
    assert_eq!(parameters[7].name.as_str(), "direction");
    assert_eq!(parameters[7].ty, GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("outgoing"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "metric");
    assert_eq!(parameters[8].ty, GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("squared_euclidean"));
    assert!(parameters[8].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.as_str(), "query_index");
    assert_eq!(columns[0].ty, GqlType::Uint64);
    assert_eq!(columns[1].name.as_str(), "node_id");
    assert_eq!(columns[1].ty, GqlType::NodeRef);
    assert_eq!(columns[2].name.as_str(), "distance");
    assert_eq!(columns[2].ty, GqlType::Float64);
}

#[test]
fn vector_search_candidate_state_expanded_ann_metadata_has_state_and_ann_args() {
    let registry = full_registry();
    let name = [
        istr("selene"),
        istr("vector_search_candidate_state_expanded_ann"),
    ];
    let metadata = registry
        .lookup(&name)
        .expect("vector_search_candidate_state_expanded_ann resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 7);
    assert_eq!(arity.maximum, 11);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "query");
    assert_eq!(parameters[2].ty, GqlType::Vector);
    assert_eq!(parameters[3].name.as_str(), "state_name");
    assert_eq!(parameters[3].ty, GqlType::String);
    assert_eq!(parameters[4].name.as_str(), "root_k");
    assert_eq!(parameters[4].ty, GqlType::Integer);
    assert_eq!(parameters[5].name.as_str(), "edge_label");
    assert_eq!(parameters[5].ty, GqlType::String);
    assert_eq!(parameters[6].name.as_str(), "k");
    assert_eq!(parameters[6].ty, GqlType::Integer);
    assert_eq!(parameters[7].name.as_str(), "operation");
    assert_eq!(parameters[7].ty, GqlType::String);
    assert_eq!(parameters[7].default_doc, Some("intersection"));
    assert!(parameters[7].default.is_some());
    assert_eq!(parameters[8].name.as_str(), "direction");
    assert_eq!(parameters[8].ty, GqlType::String);
    assert_eq!(parameters[8].default_doc, Some("outgoing"));
    assert!(parameters[8].default.is_some());
    assert_eq!(parameters[9].name.as_str(), "metric");
    assert_eq!(parameters[9].ty, GqlType::String);
    assert_eq!(parameters[9].default_doc, Some("squared_euclidean"));
    assert!(parameters[9].default.is_some());
    assert_eq!(parameters[10].name.as_str(), "ef_search");
    assert_eq!(parameters[10].ty, GqlType::Integer);
    assert!(parameters[10].nullable);
    assert_eq!(parameters[10].default_doc, Some("NULL (HNSW 64, IVF 2)"));
    assert!(parameters[10].default.is_some());

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "distance");
    assert_eq!(columns[1].ty, GqlType::Float64);
}
