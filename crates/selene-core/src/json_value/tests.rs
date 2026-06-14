use super::{JsonPathSelector, JsonValue};
use crate::db_string;

#[test]
fn human_readable_serde_preserves_json_shape() {
    let value = JsonValue::new(serde_json::json!({"b": [2, true], "a": null})).unwrap();
    let encoded = serde_json::to_value(&value).expect("JSON value serializes");
    assert_eq!(encoded, serde_json::json!({"a": null, "b": [2, true]}));
    let decoded: JsonValue = serde_json::from_value(encoded).expect("JSON value deserializes");
    assert_eq!(decoded, value);
}

#[test]
fn parse_str_rejects_duplicate_object_keys() {
    let err = JsonValue::parse_str(r#"{"a":1,"nested":{"b":1,"b":2}}"#)
        .expect_err("duplicate JSON object key rejected");

    assert_eq!(err.gqlstatus(), "22018");
    assert!(err.to_string().contains("duplicate JSON object key 'b'"));
}

#[test]
fn human_readable_serde_rejects_duplicate_object_keys() {
    let err = serde_json::from_str::<JsonValue>(r#"{"a":1,"a":2}"#)
        .expect_err("duplicate JSON object key rejected");

    assert!(err.to_string().contains("duplicate JSON object key 'a'"));
}

#[test]
fn contains_matches_nested_subset_and_array_membership() {
    let target = JsonValue::new(serde_json::json!({
        "memory": {"kind": "episodic", "score": 7},
        "tags": ["agent", "graph", {"scope": "current"}]
    }))
    .unwrap();

    assert!(
        target.contains(
            &JsonValue::new(serde_json::json!({"memory": {"kind": "episodic"}})).unwrap()
        )
    );
    assert!(target.contains(&JsonValue::new(serde_json::json!({"tags": "graph"})).unwrap()));
    assert!(target.contains(
        &JsonValue::new(serde_json::json!({"tags": [{"scope": "current"}, "agent"]})).unwrap()
    ));
    assert!(
        !target.contains(
            &JsonValue::new(serde_json::json!({"memory": {"kind": "semantic"}})).unwrap()
        )
    );
}

#[test]
fn path_exists_matches_object_keys_and_array_indexes() {
    let target = JsonValue::new(serde_json::json!({
        "memory": {"facts": [{"title": "old"}, {"title": "current"}]}
    }))
    .unwrap();
    let path = [
        JsonPathSelector::Key(db_string("memory").unwrap()),
        JsonPathSelector::Key(db_string("facts").unwrap()),
        JsonPathSelector::Index(1),
        JsonPathSelector::Key(db_string("title").unwrap()),
    ];

    assert!(target.path_exists(&path));
    assert!(target.path_exists(&[
        JsonPathSelector::Key(db_string("memory").unwrap()),
        JsonPathSelector::Key(db_string("facts").unwrap()),
        JsonPathSelector::Index(-1),
        JsonPathSelector::Key(db_string("title").unwrap()),
    ]));
    assert!(!target.path_exists(&[
        JsonPathSelector::Key(db_string("memory").unwrap()),
        JsonPathSelector::Key(db_string("facts").unwrap()),
        JsonPathSelector::UnsignedIndex(9),
    ]));
}

#[test]
fn path_value_returns_selected_json_subvalue() {
    let target = JsonValue::new(serde_json::json!({
        "memory": {
            "facts": [{"title": "old"}, {"title": "current"}],
            "score": null
        }
    }))
    .unwrap();

    let selected = target
        .path_value(&[
            JsonPathSelector::Key(db_string("memory").unwrap()),
            JsonPathSelector::Key(db_string("facts").unwrap()),
            JsonPathSelector::Index(-1),
            JsonPathSelector::Key(db_string("title").unwrap()),
        ])
        .expect("path selects a value");
    assert_eq!(selected.as_serde(), &serde_json::json!("current"));

    let null_value = target
        .path_value(&[
            JsonPathSelector::Key(db_string("memory").unwrap()),
            JsonPathSelector::Key(db_string("score").unwrap()),
        ])
        .expect("JSON null is present");
    assert_eq!(null_value.as_serde(), &serde_json::Value::Null);

    assert!(
        target
            .path_value(&[
                JsonPathSelector::Key(db_string("memory").unwrap()),
                JsonPathSelector::Key(db_string("missing").unwrap()),
            ])
            .is_none()
    );
}

#[test]
fn path_value_ref_borrows_selected_json_subvalue() {
    let target = JsonValue::new(serde_json::json!({
        "memory": {"facts": [{"title": "old"}, {"title": "current"}]}
    }))
    .unwrap();
    let path = [
        JsonPathSelector::Key(db_string("memory").unwrap()),
        JsonPathSelector::Key(db_string("facts").unwrap()),
        JsonPathSelector::Index(-1),
        JsonPathSelector::Key(db_string("title").unwrap()),
    ];

    let selected = target.path_value_ref(&path).expect("path selects a value");

    assert_eq!(selected.as_serde(), &serde_json::json!("current"));
    assert_eq!(
        selected.to_owned_json_value().as_serde(),
        &serde_json::json!("current")
    );
}

#[test]
fn path_contains_applies_containment_to_selected_subvalue() {
    let target = JsonValue::new(serde_json::json!({
        "memory": {
            "facts": [
                {"title": "old", "tags": ["archive"]},
                {"title": "current", "tags": ["agent", {"scope": "fresh"}]}
            ]
        }
    }))
    .unwrap();
    let path = [
        JsonPathSelector::Key(db_string("memory").unwrap()),
        JsonPathSelector::Key(db_string("facts").unwrap()),
        JsonPathSelector::Index(-1),
    ];

    assert!(target.path_contains(
        &path,
        &JsonValue::new(serde_json::json!({"tags": [{"scope": "fresh"}]})).unwrap()
    ));
    assert!(!target.path_contains(
        &path,
        &JsonValue::new(serde_json::json!({"tags": "archive"})).unwrap()
    ));
    assert!(!target.path_contains(
        &[JsonPathSelector::Key(db_string("missing").unwrap())],
        &JsonValue::new(serde_json::json!(null)).unwrap()
    ));
}

#[test]
fn merge_patch_matches_rfc7396_core_cases() {
    for (target, patch, expected) in [
        (r#"{"a":"b"}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#),
        (r#"{"a":"b"}"#, r#"{"b":"c"}"#, r#"{"a":"b","b":"c"}"#),
        (r#"{"a":"b"}"#, r#"{"a":null}"#, r#"{}"#),
        (r#"{"a":"b","b":"c"}"#, r#"{"a":null}"#, r#"{"b":"c"}"#),
        (r#"{"a":["b"]}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#),
        (r#"{"a":"c"}"#, r#"{"a":["b"]}"#, r#"{"a":["b"]}"#),
        (
            r#"{"a":{"b":"c"}}"#,
            r#"{"a":{"b":"d","c":null}}"#,
            r#"{"a":{"b":"d"}}"#,
        ),
        (r#"{"a":[{"b":"c"}]}"#, r#"{"a":[1]}"#, r#"{"a":[1]}"#),
        (r#"["a","b"]"#, r#"["c","d"]"#, r#"["c","d"]"#),
        (r#"{"a":"b"}"#, r#"["c"]"#, r#"["c"]"#),
        (r#"{"a":"foo"}"#, r#"null"#, r#"null"#),
        (r#"{"a":"foo"}"#, r#""bar""#, r#""bar""#),
        (r#"{"e":null}"#, r#"{"a":1}"#, r#"{"a":1,"e":null}"#),
    ] {
        let target = JsonValue::parse_str(target).expect("target JSON parses");
        let patch = JsonValue::parse_str(patch).expect("patch JSON parses");
        let expected = JsonValue::parse_str(expected).expect("expected JSON parses");

        assert_eq!(
            target.merge_patch(&patch).expect("merge patch succeeds"),
            expected
        );
    }
}
