//! Bounded catalog mutation sequences checked against a simple set model.
//!
//! Operations 0-3 use the Rust lifecycle API; operations 4-7 issue the same
//! commands as GQL database-catalog statements through a selected fixture
//! session. Both arms must agree with one set model.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use selene_db::{CreatePolicy, Database, DropPolicy, ExecutionOutcome, ObjectPath, SchemaPath};

const OMITTED: ExecutionOutcome = ExecutionOutcome::SUCCESSFUL_OMITTED;

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).unwrap()
}

fn graph(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn bounded_mutation_sequences_match_model(
        commands in prop::collection::vec((0_u8..8, 0_u8..4, 0_u8..4), 0..40),
    ) {
        let database = Database::builder().build();
        let catalog = database.catalog();
        catalog
            .create_schema(&schema("control"), CreatePolicy::Strict)
            .unwrap();
        catalog
            .create_graph(&graph("control", "session"), None, CreatePolicy::Strict)
            .unwrap();
        let session = database.session(&graph("control", "session")).unwrap();
        let mut model: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for (operation, schema_index, graph_index) in commands {
            let schema_name = format!("s{schema_index}");
            let graph_name = format!("g{graph_index}");
            match operation {
                0 => {
                    catalog
                        .create_schema(&schema(&schema_name), CreatePolicy::IfNotExists)
                        .unwrap();
                    model.entry(schema_name.clone()).or_default();
                }
                1 => {
                    let has_children = model
                        .get(&schema_name)
                        .is_some_and(|graphs| !graphs.is_empty());
                    let result = catalog.drop_schema(&schema(&schema_name), DropPolicy::IfExists);
                    if has_children {
                        prop_assert!(result.is_err());
                    } else {
                        prop_assert!(result.is_ok());
                        model.remove(&schema_name);
                    }
                }
                2 => {
                    let result = catalog.create_graph(
                        &graph(&schema_name, &graph_name),
                        None,
                        CreatePolicy::IfNotExists,
                    );
                    if let Some(graphs) = model.get_mut(&schema_name) {
                        prop_assert!(result.is_ok());
                        graphs.insert(graph_name.clone());
                    } else {
                        prop_assert!(result.is_err());
                    }
                }
                3 => {
                    let result = catalog.drop_graph(
                        &graph(&schema_name, &graph_name),
                        DropPolicy::IfExists,
                    );
                    if let Some(graphs) = model.get_mut(&schema_name) {
                        prop_assert!(result.is_ok());
                        graphs.remove(&graph_name);
                    } else {
                        prop_assert!(result.is_err());
                    }
                }
                4 => {
                    let outcome = session
                        .execute(&format!("CREATE SCHEMA IF NOT EXISTS /{schema_name}"))
                        .unwrap();
                    prop_assert_eq!(outcome, OMITTED);
                    model.entry(schema_name.clone()).or_default();
                }
                5 => {
                    let has_children = model
                        .get(&schema_name)
                        .is_some_and(|graphs| !graphs.is_empty());
                    let result = session.execute(&format!("DROP SCHEMA IF EXISTS /{schema_name}"));
                    if has_children {
                        prop_assert!(result.is_err());
                    } else {
                        prop_assert_eq!(result.unwrap(), OMITTED);
                        model.remove(&schema_name);
                    }
                }
                6 => {
                    let result = session.execute(&format!(
                        "CREATE GRAPH IF NOT EXISTS /{schema_name}/{graph_name} ANY"
                    ));
                    if let Some(graphs) = model.get_mut(&schema_name) {
                        prop_assert_eq!(result.unwrap(), OMITTED);
                        graphs.insert(graph_name.clone());
                    } else {
                        prop_assert!(result.is_err());
                    }
                }
                7 => {
                    let result = session.execute(&format!(
                        "DROP GRAPH IF EXISTS /{schema_name}/{graph_name}"
                    ));
                    if let Some(graphs) = model.get_mut(&schema_name) {
                        let expected = if graphs.remove(&graph_name) {
                            OMITTED
                        } else {
                            ExecutionOutcome::GRAPH_NOT_FOUND_OMITTED
                        };
                        prop_assert_eq!(result.unwrap(), expected);
                    } else {
                        prop_assert!(result.is_err());
                    }
                }
                _ => unreachable!(),
            }

            let snapshot = catalog.snapshot();
            let actual_schemas = snapshot
                .schemas()
                .unwrap()
                .into_iter()
                .filter_map(|descriptor| {
                    let name = descriptor.path.schema().canonical();
                    (name != "control").then(|| name.to_owned())
                })
                .collect::<BTreeSet<_>>();
            prop_assert_eq!(actual_schemas, model.keys().cloned().collect());
            for (name, expected_graphs) in &model {
                let actual_graphs = snapshot
                    .graphs(&schema(name))
                    .unwrap()
                    .into_iter()
                    .map(|descriptor| descriptor.path.object().canonical().to_owned())
                    .collect::<BTreeSet<_>>();
                prop_assert_eq!(&actual_graphs, expected_graphs);
            }
        }
    }
}
