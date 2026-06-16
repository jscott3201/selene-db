//! ISO `<typed>` marker coverage for catalog property declarations.

use selene_core::{PropertyValueType, Value};
use selene_gql::parse;
use selene_graph::PropertyElementType;

use super::{empty_closed_graph, planned, run_write};

#[test]
fn property_type_accepts_optional_colon_and_typed_markers() {
    let graph = empty_closed_graph(3750);
    let ddl =
        planned("CREATE NODE TYPE :Doc (title TYPED STRING, views INTEGER, tags :: LIST<STRING>)");

    run_write(&graph, &ddl)
        .expect("catalog DDL executes")
        .1
        .expect("catalog commit succeeds");

    let graph_type = graph.graph_type().expect("closed graph type");
    let properties = &graph_type.node_types[0].properties;
    assert_eq!(properties[0].name.as_str(), "title");
    assert_eq!(properties[0].value_type, PropertyValueType::String);
    assert_eq!(properties[1].name.as_str(), "views");
    assert_eq!(properties[1].value_type, PropertyValueType::Int);
    assert_eq!(properties[2].value_type, PropertyValueType::List);
    assert!(matches!(
        properties[2].list_element_type.as_ref(),
        Some(PropertyElementType::Scalar(PropertyValueType::String))
    ));

    let (table, outcome) = run_write(&graph, &planned("SHOW NODE TYPES")).expect("show executes");
    outcome.expect("show commit succeeds");
    let Value::String(definition) = &table.rows()[0].values()[1] else {
        panic!("definition is a string");
    };
    assert_eq!(
        definition.as_str(),
        "CREATE NODE TYPE :Doc (title :: STRING, views :: INTEGER, tags :: LIST<STRING>)"
    );
    parse(definition.as_str()).expect("canonical SHOW definition reparses");
}
