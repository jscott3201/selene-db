//! BRIEF-120 procedure metadata coverage.

use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{BindingTable, ProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;
use selene_pack::ProcedurePackRegistry;
use selene_vector_pack::VectorPack;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn full_registry() -> ProcedurePackRegistry {
    let vector = VectorPack::new();
    let algorithms = AlgorithmsPack::new();
    ProcedurePackRegistry::builder()
        .with_builtins()
        .with_external_pack(vector.external_pack())
        .with_external_pack(algorithms.external_pack())
        .build()
        .expect("full default registry builds")
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
            Some(Value::ExternalString(value)) => value.as_ref().to_owned(),
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
fn default_registry_exposes_non_empty_metadata_for_all_36_procedures() {
    let registry = full_registry();
    let procedures = registry.iter_handles().collect::<Vec<_>>();

    assert_eq!(procedures.len(), 36);
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
fn show_procedures_exposes_seven_columns_and_zero_arg_description() {
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
            "capability_required",
        ]
    );
    assert_eq!(table.row_count(), 36);

    let names = column_strings(&table, "name");
    let descriptions = column_strings(&table, "description");
    let health = names
        .iter()
        .position(|name| name == "selene.health")
        .expect("selene.health is registered");
    assert_eq!(descriptions[health], "Report basic graph health counters.");
}
