use super::*;

#[test]
fn json_parse_returns_canonical_json_value() {
    let value = json_value(r#"RETURN json('{"b":[2,true],"a":null}') AS value"#);
    assert_eq!(value.to_canonical_string(), r#"{"a":null,"b":[2,true]}"#);
}

#[test]
fn json_stringify_and_cast_to_string_use_canonical_form() {
    assert_eq!(
        string_value(r#"RETURN json_stringify(json('{"b":2,"a":1}')) AS value"#),
        r#"{"a":1,"b":2}"#
    );
    assert_eq!(
        string_value(r#"RETURN CAST(json('{"b":2,"a":1}') AS STRING) AS value"#),
        r#"{"a":1,"b":2}"#
    );
}

#[test]
fn json_type_reports_top_level_shape() {
    for (source, expected) in [
        (r#"RETURN json_type(json('null')) AS value"#, "null"),
        (r#"RETURN json_type(json('true')) AS value"#, "boolean"),
        (r#"RETURN json_type(json('1.5')) AS value"#, "number"),
        (r#"RETURN json_type(json('"agent"')) AS value"#, "string"),
        (r#"RETURN json_type(json('[1]')) AS value"#, "array"),
        (r#"RETURN json_type(json('{"a":1}')) AS value"#, "object"),
    ] {
        assert_eq!(string_value(source), expected, "{source}");
    }
}

#[test]
fn json_array_constructs_canonical_json_values() {
    let value = json_value(
        r#"RETURN json_array(7, TRUE, NULL, 'agent', json('{"b":2,"a":1}'), [1, 2, NULL]) AS value"#,
    );

    assert_eq!(
        value.to_canonical_string(),
        r#"[7,true,null,"agent",{"a":1,"b":2},[1,2,null]]"#
    );
    assert_eq!(
        json_value("RETURN json_array() AS value").to_canonical_string(),
        "[]"
    );
}

#[test]
fn json_object_constructs_canonical_json_values() {
    let value = json_value(
        "RETURN json_object(\
         'kind', 'episodic', \
         'score', 7, \
         'meta', json_object('current', TRUE, 'tags', json_array('agent', 'graph'))) AS value",
    );

    assert_eq!(
        value.to_canonical_string(),
        r#"{"kind":"episodic","meta":{"current":true,"tags":["agent","graph"]},"score":7}"#
    );
    assert_eq!(
        json_value("RETURN json_object() AS value").to_canonical_string(),
        "{}"
    );
}

#[test]
fn json_object_rejects_duplicate_keys() {
    assert_status("RETURN json_object('a', 1, 'a', 2) AS value", "22G03");
}

#[test]
fn json_parse_rejects_duplicate_object_keys() {
    for source in [
        r#"RETURN json('{"a":1,"a":2}') AS value"#,
        r#"RETURN json_parse('{"nested":{"a":1,"a":2}}') AS value"#,
        r#"RETURN CAST('{"a":1,"a":2}' AS JSON) AS value"#,
    ] {
        assert_status(source, "22018");
    }
}

#[test]
fn json_array_length_counts_array_elements() {
    assert_eq!(
        int_value(r#"RETURN json_array_length(json('[1,{"a":2},null]')) AS value"#),
        3
    );
    assert_eq!(
        single_value("RETURN json_array_length(NULL) AS value", "value"),
        Value::Null
    );
}

#[test]
fn json_object_keys_returns_sorted_string_list() {
    assert_eq!(
        string_list_value(r#"RETURN json_object_keys(json('{"b":2,"a":1,"c":null}')) AS value"#),
        vec!["a", "b", "c"]
    );
    assert_eq!(
        single_value("RETURN json_object_keys(NULL) AS value", "value"),
        Value::Null
    );
}

#[test]
fn json_contains_matches_nested_object_and_array_subsets() {
    for source in [
        r#"RETURN json_contains(json('{"memory":{"kind":"episodic","score":7},"tags":["agent","graph"]}'), json('{"memory":{"kind":"episodic"}}')) AS value"#,
        r#"RETURN json_contains(json('["agent","graph",{"kind":"memory","score":7}]'), json('[{"kind":"memory"},"agent"]')) AS value"#,
        r#"RETURN json_contains(json('["agent","graph"]'), json('"graph"')) AS value"#,
        r#"RETURN json_contains(json('{"b":2,"a":1}'), json('{"a":1,"b":2}')) AS value"#,
    ] {
        assert!(bool_value(source), "{source}");
    }
}

#[test]
fn json_contains_rejects_missing_or_mismatched_candidates() {
    for source in [
        r#"RETURN json_contains(json('{"memory":{"kind":"semantic"}}'), json('{"memory":{"kind":"episodic"}}')) AS value"#,
        r#"RETURN json_contains(json('{"memory":{"kind":"episodic"}}'), json('{"memory":{"score":7}}')) AS value"#,
        r#"RETURN json_contains(json('["agent","graph"]'), json('"memory"')) AS value"#,
        r#"RETURN json_contains(json('{"a":1}'), json('[1]')) AS value"#,
    ] {
        assert!(!bool_value(source), "{source}");
    }
}

#[test]
fn json_contains_propagates_sql_null_arguments() {
    for source in [
        r#"RETURN json_contains(NULL, json('{}')) AS value"#,
        r#"RETURN json_contains(json('{}'), NULL) AS value"#,
    ] {
        assert_eq!(single_value(source, "value"), Value::Null, "{source}");
    }
}

#[test]
fn json_get_selects_objects_and_arrays() {
    let value = json_value(
        r#"RETURN json_get(json('{"memory":{"score":7,"kind":"episodic"}}'), 'memory') AS value"#,
    );
    assert_eq!(
        value.to_canonical_string(),
        r#"{"kind":"episodic","score":7}"#
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('{"name":"alpha"}'), 'name') AS value"#),
        "alpha"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('{"memory":{"score":7}}'), 'memory') AS value"#),
        r#"{"score":7}"#
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), 1) AS value"#),
        "20"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), -1) AS value"#),
        "30"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), CAST(1 AS INT128)) AS value"#),
        "20"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), CAST(1 AS UINT128)) AS value"#),
        "20"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), 1M) AS value"#),
        "20"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), -1M) AS value"#),
        "30"
    );
}

#[test]
fn json_get_scalar_returns_native_leaf_values() {
    for (source, expected) in [
        (
            r#"RETURN json_get_scalar(json('{"ok":true}'), 'ok') AS value"#,
            Value::Bool(true),
        ),
        (
            r#"RETURN json_get_scalar(json('{"score":7}'), 'score') AS value"#,
            Value::Int(7),
        ),
        (
            r#"RETURN json_get_scalar(json('{"id":18446744073709551615}'), 'id') AS value"#,
            Value::Uint(u64::MAX),
        ),
        (
            r#"RETURN json_get_scalar(json('{"name":"alpha"}'), 'name') AS value"#,
            Value::String(db_string("alpha")),
        ),
        (
            r#"RETURN json_get_scalar(json('{"a":null}'), 'a') AS value"#,
            Value::Null,
        ),
    ] {
        assert_eq!(single_value(source, "value"), expected, "{source}");
    }
    match single_value(
        r#"RETURN json_get_path_scalar(json('{"memory":{"score":1.5,"current":true}}'), 'memory', 'score') AS value"#,
        "value",
    ) {
        Value::Float(value) => assert_eq!(value, 1.5),
        other => panic!("expected FLOAT value, got {other:?}"),
    }
    assert_eq!(
        single_value(
            r#"RETURN json_get_path_scalar(json('{"memory":{"current":true}}'), 'memory', 'current') AS value"#,
            "value",
        ),
        Value::Bool(true)
    );
}

#[test]
fn json_get_path_selects_nested_objects_and_arrays() {
    let value = json_value(
        r#"RETURN json_get_path(json('{"memory":{"events":[{"score":7},{"score":9}]}}'), 'memory', 'events', 1, 'score') AS value"#,
    );
    assert_eq!(value.to_canonical_string(), "9");
    assert_eq!(
        string_value(
            r#"RETURN json_get_path_text(json('{"events":[{"kind":"semantic"},{"kind":"episodic"}]}'), 'events', -1, 'kind') AS value"#,
        ),
        "episodic"
    );
    let null_value = json_value(
        r#"RETURN json_get_path(json('{"memory":{"score":null}}'), 'memory', 'score') AS value"#,
    );
    assert_eq!(null_value.to_canonical_string(), "null");
    assert_eq!(
        string_value(
            r#"RETURN json_get_path_text(json('{"a":{"b":[true,{"c":[1,2]}]}}'), 'a', 'b', 1, 'c') AS value"#,
        ),
        "[1,2]"
    );
}

#[test]
fn json_get_path_returns_sql_null_for_absent_or_inapplicable_paths() {
    for source in [
        r#"RETURN json_get_path(json('{"a":{"b":1}}'), 'a', 'missing') AS value"#,
        r#"RETURN json_get_path(json('{"a":{"b":1}}'), 'a', NULL, 'b') AS value"#,
        r#"RETURN json_get_path(NULL, 'a', 'b') AS value"#,
        r#"RETURN json_get_path(json('[{"a":1}]'), 99, 'a') AS value"#,
        r#"RETURN json_get(json('[1,2]'), 9999999999999999999999999999M) AS value"#,
        r#"RETURN json_get_path(json('{"a":1}'), 'a', 'b') AS value"#,
        r#"RETURN json_get_path_text(json('{"a":{"b":null}}'), 'a', 'b') AS value"#,
    ] {
        assert_eq!(single_value(source, "value"), Value::Null, "{source}");
    }
}

#[test]
fn json_get_path_reports_data_exceptions_for_bad_selectors() {
    for source in [
        r#"RETURN json_get_path(json('{"a":1}'), 7) AS value"#,
        r#"RETURN json_get_path(json('[1,2]'), 'bad') AS value"#,
        r#"RETURN json_get(json('[1,2]'), 1.5M) AS value"#,
        r#"RETURN json_get(json('[1,2]'), -1.5M) AS value"#,
        r#"RETURN json_get_path(7, 'a') AS value"#,
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn json_has_path_distinguishes_json_null_from_missing_paths() {
    for source in [
        r#"RETURN json_has_path(json('{"a":{"b":1}}'), 'a', 'b') AS value"#,
        r#"RETURN json_has_path(json('{"a":{"b":null}}'), 'a', 'b') AS value"#,
        r#"RETURN json_has_path(json('{"a":[1,2,3]}'), 'a', -1) AS value"#,
        r#"RETURN json_has_path(json('[1,2,3]'), CAST(1 AS INT128)) AS value"#,
        r#"RETURN json_has_path(json('[1,2,3]'), 1M) AS value"#,
    ] {
        assert!(bool_value(source), "{source}");
    }
    for source in [
        r#"RETURN json_has_path(json('{"a":{"b":1}}'), 'a', 'missing') AS value"#,
        r#"RETURN json_has_path(json('[{"a":1}]'), 99, 'a') AS value"#,
        r#"RETURN json_has_path(json('{"a":1}'), 'a', 'b') AS value"#,
    ] {
        assert!(!bool_value(source), "{source}");
    }
    for source in [
        r#"RETURN json_has_path(NULL, 'a') AS value"#,
        r#"RETURN json_has_path(json('{"a":1}'), NULL) AS value"#,
    ] {
        assert_eq!(single_value(source, "value"), Value::Null, "{source}");
    }
}

#[test]
fn json_has_path_reports_data_exceptions_for_bad_selectors() {
    for source in [
        r#"RETURN json_has_path(json('{"a":1}'), 7) AS value"#,
        r#"RETURN json_has_path(json('[1,2]'), 'bad') AS value"#,
        r#"RETURN json_has_path(7, 'a') AS value"#,
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn json_get_path_enforces_selector_depth_cap() {
    let selectors = vec!["'a'"; 65].join(", ");
    let source = format!("RETURN json_get_path(json('{{}}'), {selectors}) AS value");
    let err = execute_read_result(&source).expect_err("over-wide JSON path should fail");
    assert!(matches!(
        err,
        ExecutorError::FunctionArityMismatch {
            expected: "2 to 65",
            actual: 66,
            ..
        }
    ));
}

#[test]
fn json_get_returns_sql_null_for_absent_or_inapplicable_paths() {
    for source in [
        r#"RETURN json_get(json('{"a":1}'), 'missing') AS value"#,
        r#"RETURN json_get(json('{"a":1}'), NULL) AS value"#,
        r#"RETURN json_get(NULL, 'a') AS value"#,
        r#"RETURN json_get(json('[1,2]'), 99) AS value"#,
        r#"RETURN json_get(json('"not-container"'), 'a') AS value"#,
        r#"RETURN json_get_text(json('{"a":null}'), 'a') AS value"#,
        r#"RETURN json_get_scalar(json('{"a":1}'), 'missing') AS value"#,
        r#"RETURN json_get_path_scalar(json('{"a":{"b":null}}'), 'a', 'b') AS value"#,
    ] {
        assert_eq!(single_value(source, "value"), Value::Null, "{source}");
    }
}

#[test]
fn json_equality_uses_value_semantics_but_ordering_is_rejected() {
    assert_eq!(
        single_value(
            r#"RETURN json('{"b":2,"a":1}') = json('{"a":1,"b":2}') AS value"#,
            "value",
        ),
        Value::Bool(true)
    );
    assert_status(
        r#"RETURN json('{"a":1}') < json('{"a":2}') AS value"#,
        "22G04",
    );
}

#[test]
fn json_functions_report_data_exceptions_for_bad_inputs() {
    for source in [
        "RETURN json('{') AS value",
        "RETURN CAST('[' AS JSON) AS value",
    ] {
        assert_status(source, "22018");
    }
    for source in [
        "RETURN json(7) AS value",
        "RETURN json_stringify(7) AS value",
        "RETURN json_type(7) AS value",
        "RETURN json_object('a') AS value",
        "RETURN json_object(NULL, 1) AS value",
        "RETURN json_object(7, 1) AS value",
        "RETURN json_array(DATE '2026-01-01') AS value",
        "RETURN json_array(CAST('123.45' AS DECIMAL)) AS value",
        "RETURN json_array_length(7) AS value",
        "RETURN json_array_length(json('{}')) AS value",
        "RETURN json_object_keys(7) AS value",
        "RETURN json_object_keys(json('[]')) AS value",
        "RETURN json_contains(7, json('{}')) AS value",
        "RETURN json_contains(json('{}'), 7) AS value",
        "RETURN json_merge_patch(7, json('{}')) AS value",
        "RETURN json_merge_patch(json('{}'), 7) AS value",
        "RETURN json_patch(7, json('[]')) AS value",
        "RETURN json_patch(json('{}'), 7) AS value",
        r#"RETURN json_get(json('{"a":1}'), 7) AS value"#,
        r#"RETURN json_get(json('[1,2]'), 'bad') AS value"#,
        r#"RETURN json_get_scalar(json('{"a":{}}'), 'a') AS value"#,
        r#"RETURN json_get_path_scalar(json('{"a":[1]}'), 'a') AS value"#,
        "RETURN CAST(7 AS JSON) AS value",
        "RETURN CAST(json('{}') AS INTEGER) AS value",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn json_array_rejects_more_than_sixty_four_arguments() {
    let args = (0..65).map(|_| "NULL").collect::<Vec<_>>().join(", ");
    let source = format!("RETURN json_array({args}) AS value");
    let err = execute_read_result(&source).expect_err("over-wide JSON constructor should fail");

    assert!(matches!(
        err,
        ExecutorError::FunctionArityMismatch {
            ref name,
            expected: "variable",
            actual: 65,
            ..
        } if name == "json_array"
    ));
}
