use super::*;

#[test]
fn json_feature_flags_cover_functions_and_type_names() {
    for source in [
        r#"RETURN json('{"a":1}') AS value"#,
        r#"RETURN json_parse('{"a":1}') AS value"#,
        r#"RETURN json_stringify(json('{"a":1}')) AS value"#,
        r#"RETURN json_type(json('{"a":1}')) AS value"#,
        "RETURN json_array(1, 'two') AS value",
        "RETURN json_object('a', 1) AS value",
        r#"RETURN json_array_length(json('[1,2]')) AS value"#,
        r#"RETURN json_object_keys(json('{"a":1}')) AS value"#,
        r#"RETURN json_contains(json('{"a":1}'), json('{"a":1}')) AS value"#,
        r#"RETURN json_merge_patch(json('{"a":1}'), json('{"b":2}')) AS value"#,
        r#"RETURN json_patch(json('{"a":1}'), json('[{"op":"add","path":"/b","value":2}]')) AS value"#,
        r#"RETURN json_get(json('{"a":1}'), 'a') AS value"#,
        r#"RETURN json_get_text(json('{"a":1}'), 'a') AS value"#,
        r#"RETURN json_get_scalar(json('{"a":1}'), 'a') AS value"#,
        r#"RETURN json_get_path(json('{"a":{"b":1}}'), 'a', 'b') AS value"#,
        r#"RETURN json_get_path_text(json('{"a":{"b":1}}'), 'a', 'b') AS value"#,
        r#"RETURN json_get_path_scalar(json('{"a":{"b":1}}'), 'a', 'b') AS value"#,
        r#"RETURN json_has_path(json('{"a":{"b":1}}'), 'a', 'b') AS value"#,
        "RETURN NULL IS TYPED JSON AS value",
        "CREATE NODE TYPE :Thing (payload :: JSON)",
    ] {
        assert_feature_recorded(source);
    }
}

#[test]
fn json_typed_parameters_accept_json_values_and_reject_other_values() {
    let name = db_string("doc");
    let statement = typed_parameter_statement(name.clone());
    let analyzed =
        selene_gql::analyze(statement, &EmptyProcedureRegistry, None).expect("statement analyzes");
    let plan = selene_gql::plan(&analyzed, &EmptyProcedureRegistry).expect("statement plans");
    let graph = SharedGraph::new(GraphId::new(14_001));
    let mut session = Session::new(&graph);

    session.bind_parameter(
        name.clone(),
        Value::Json(CoreJsonValue::new(serde_json::json!({"a": 1})).expect("JSON is valid")),
    );
    selene_gql::execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect("JSON parameter accepts matching value");

    session.bind_parameter(name.clone(), Value::String(db_string("{}")));
    let err = selene_gql::execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("JSON parameter rejects string value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name: err_name,
            ref expected,
            actual: "STRING",
            ..
        } if err_name == name && expected == "JSON"
    ));
}

#[test]
fn is_typed_json_matches_json_values() {
    assert_eq!(
        single_value(r#"RETURN json('{"a":1}') IS TYPED JSON AS value"#, "value"),
        Value::Bool(true)
    );
    assert_eq!(
        single_value(r#"RETURN '{"a":1}' IS TYPED JSON AS value"#, "value"),
        Value::Bool(false)
    );
}

#[test]
fn json_node_type_round_trips_catalog_and_execution() {
    let graph = empty_closed_graph(14_002);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Thing (payload :: JSON NOT NULL)",
            &EmptyProcedureRegistry,
        )
        .expect("JSON node type creates");

    let graph_type = graph.graph_type().expect("graph has type");
    let declaration = &graph_type.node_types[0].properties[0];
    assert_eq!(declaration.value_type, PropertyValueType::Json);
    assert!(declaration.required);

    session
        .execute_source(
            r#"INSERT (:Thing {payload: json('{"name":"alpha","score":7}')})"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON property inserts");
    let output = session
        .execute_source(
            "MATCH (n:Thing) RETURN json_get_text(n.payload, 'score') AS value",
            &EmptyProcedureRegistry,
        )
        .expect("JSON property reads");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "value"),
        vec![Value::String(db_string("7"))]
    );

    session
        .execute_source(
            r#"MATCH (n:Thing) SET n.payload = CAST('{"name":"beta","meta":{"seen":true}}' AS JSON) FINISH"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON property updates");
    let output = session
        .execute_source(
            "MATCH (n:Thing) RETURN json_get_path_text(n.payload, 'name') AS name, json_has_path(n.payload, 'meta', 'seen') AS seen",
            &EmptyProcedureRegistry,
        )
        .expect("updated JSON property reads");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "name"),
        vec![Value::String(db_string("beta"))]
    );
    assert_eq!(column_values(&table, "seen"), vec![Value::Bool(true)]);

    session
        .execute_source(
            "MATCH (n:Thing) SET n.payload = 'not-json' FINISH",
            &EmptyProcedureRegistry,
        )
        .expect_err("JSON property rejects non-JSON assignment");
    session
        .execute_source(
            "MATCH (n:Thing) REMOVE n.payload FINISH",
            &EmptyProcedureRegistry,
        )
        .expect_err("required JSON property cannot be removed");

    let output = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW NODE TYPES executes");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "definition"),
        vec![Value::String(db_string(
            "CREATE NODE TYPE :Thing (payload :: JSON NOT NULL)"
        ))]
    );
}

#[test]
fn json_node_type_default_materializes_and_round_trips() {
    let graph = empty_closed_graph(14_003);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            r#"CREATE NODE TYPE :Thing (payload :: JSON NOT NULL DEFAULT '{"b":2,"a":"don''t"}')"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON node type with default creates");

    session
        .execute_source("INSERT (:Thing)", &EmptyProcedureRegistry)
        .expect("JSON default materializes on insert");
    let output = session
        .execute_source(
            "MATCH (n:Thing) RETURN json_get_text(n.payload, 'a') AS value",
            &EmptyProcedureRegistry,
        )
        .expect("defaulted JSON property reads");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "value"),
        vec![Value::String(db_string("don't"))]
    );

    let output = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW NODE TYPES executes");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "definition"),
        vec![Value::String(db_string(
            r#"CREATE NODE TYPE :Thing (payload :: JSON NOT NULL DEFAULT '{"a":"don''t","b":2}')"#
        ))]
    );
}
