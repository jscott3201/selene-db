//! Profile format, validation, determinism, and freshness regression tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use selene_profile::{
    ANNEX_B_REGISTER, ClaimState, NOT_SUPPORTED_RATIONALE, REFERENCED_FEATURES, SUPPORTED_FEATURES,
    check_repository, parse_profile, render_outputs, write_repository,
};
use serde_json::{Value, json};

const SOURCE: &str = include_str!("../../../spec/gql-profile/profile.json");
const SCHEMA: &str = include_str!("../../../spec/gql-profile/schema.json");

fn source_value() -> Value {
    serde_json::from_str(SOURCE).expect("checked-in profile is JSON")
}

fn parse_value(value: &Value) -> Result<selene_profile::ValidatedProfile, String> {
    parse_profile(&serde_json::to_string(value).expect("fixture serializes"))
        .map_err(|error| error.to_string())
}

fn feature_mut(value: &mut Value, index: usize) -> &mut serde_json::Map<String, Value> {
    value["features"][index]
        .as_object_mut()
        .expect("feature fixture is an object")
}

#[test]
fn checked_in_profile_loads_and_preserves_seed_contract() {
    let profile = parse_profile(SOURCE).expect("checked-in profile validates");
    assert_eq!(profile.hash().len(), 64);
    assert_eq!(profile.profile().features.len(), 165);
    assert_eq!(profile.profile().implementation_extensions.len(), 11);
    assert_eq!(REFERENCED_FEATURES.len(), 176);
    assert_eq!(SUPPORTED_FEATURES.len(), 143);
    assert_eq!(NOT_SUPPORTED_RATIONALE.len(), 32);
    assert_eq!(ANNEX_B_REGISTER.len(), 34);
    assert!(SUPPORTED_FEATURES.windows(4).any(|window| {
        window
            == [
                selene_profile::FeatureId::GA01,
                selene_profile::FeatureId::GA05,
                selene_profile::FeatureId::GA06,
                selene_profile::FeatureId::GA07,
            ]
    }));

    assert!(
        profile
            .profile()
            .features
            .iter()
            .all(|feature| feature.claim_state != ClaimState::Claimed)
    );
    assert!(
        profile
            .profile()
            .features
            .iter()
            .all(|feature| { !feature.id.as_str().starts_with("IM_") })
    );
}

#[test]
fn closed_decode_rejects_missing_and_extra_fields() {
    let mut missing = source_value();
    feature_mut(&mut missing, 0).remove("name");
    assert!(
        parse_value(&missing)
            .unwrap_err()
            .contains("missing field `name`")
    );

    let mut extra = source_value();
    feature_mut(&mut extra, 0).insert("surprise".to_owned(), json!(true));
    assert!(
        parse_value(&extra)
            .unwrap_err()
            .contains("unknown field `surprise`")
    );

    let mut missing_order = source_value();
    missing_order
        .as_object_mut()
        .expect("profile fixture is an object")
        .remove("supported_feature_order");
    assert!(
        parse_value(&missing_order)
            .unwrap_err()
            .contains("missing field `supported_feature_order`")
    );
}

#[test]
fn malformed_state_and_identifier_fail() {
    let mut state = source_value();
    feature_mut(&mut state, 0).insert("claim_state".to_owned(), json!("implemented"));
    let error = parse_value(&state).unwrap_err();
    assert!(error.contains("unknown variant `implemented`"), "{error}");

    let mut identity = source_value();
    feature_mut(&mut identity, 0).insert("id".to_owned(), json!("g002"));
    let error = parse_value(&identity).unwrap_err();
    assert!(error.contains("malformed feature ID g002"), "{error}");
}

#[test]
fn duplicate_ids_and_runtime_orders_fail() {
    let mut duplicate = source_value();
    let first = duplicate["features"][0].clone();
    duplicate["features"]
        .as_array_mut()
        .expect("features")
        .push(first);
    let error = parse_value(&duplicate).unwrap_err();
    assert!(error.contains("duplicate feature ID"), "{error}");

    let mut order = source_value();
    let first_order = order["features"][0]["runtime_order"].clone();
    feature_mut(&mut order, 1).insert("runtime_order".to_owned(), first_order);
    let error = parse_value(&order).unwrap_err();
    assert!(error.contains("duplicate runtime_order"), "{error}");
}

#[test]
fn supported_compatibility_order_is_unique_known_and_complete() {
    let mut duplicate = source_value();
    let first = duplicate["supported_feature_order"][0].clone();
    duplicate["supported_feature_order"]
        .as_array_mut()
        .expect("supported order")
        .push(first);
    let error = parse_value(&duplicate).unwrap_err();
    assert!(
        error.contains("duplicate supported compatibility ID"),
        "{error}"
    );

    let mut unknown = source_value();
    unknown["supported_feature_order"][0] = json!("IM_UNKNOWN");
    let error = parse_value(&unknown).unwrap_err();
    assert!(
        error.contains("supported_feature_order references unknown runtime ID IM_UNKNOWN"),
        "{error}"
    );

    let mut incomplete = source_value();
    incomplete["supported_feature_order"]
        .as_array_mut()
        .expect("supported order")
        .pop();
    let error = parse_value(&incomplete).unwrap_err();
    assert!(
        error.contains(
            "supported_feature_order must contain every runtime-supported feature and extension exactly once"
        ),
        "{error}"
    );
}

#[test]
fn extension_ids_must_be_rust_identifiers() {
    for invalid in ["IM_BAD-ID", "IM_BAD.ID"] {
        let mut value = source_value();
        value["implementation_extensions"][0]["id"] = json!(invalid);
        let error = parse_value(&value).unwrap_err();
        assert!(
            error.contains(&format!("malformed implementation extension ID {invalid}")),
            "{error}"
        );
    }
}

#[test]
fn unknown_implication_evidence_and_applicability_references_fail() {
    let mut implication = source_value();
    implication["implications"] = json!([{
        "id": "IMP-UNKNOWN",
        "source": "G002",
        "target": "G999",
        "clause_anchors": [],
        "evidence": []
    }]);
    let error = parse_value(&implication).unwrap_err();
    assert!(
        error.contains("references unknown feature ID G999"),
        "{error}"
    );

    let mut evidence = source_value();
    feature_mut(&mut evidence, 0).insert("evidence".to_owned(), json!(["EVID-MISSING"]));
    let error = parse_value(&evidence).unwrap_err();
    assert!(
        error.contains("references unknown evidence ID EVID-MISSING"),
        "{error}"
    );

    let mut applicability = source_value();
    feature_mut(&mut applicability, 0).insert("applicability".to_owned(), json!("APP-MISSING"));
    let error = parse_value(&applicability).unwrap_err();
    assert!(
        error.contains("references unknown applicability ID APP-MISSING"),
        "{error}"
    );
}

#[test]
fn other_dangling_references_fail() {
    let mut clause = source_value();
    feature_mut(&mut clause, 0).insert("clause_anchors".to_owned(), json!(["CLAUSE-MISSING"]));
    let error = parse_value(&clause).unwrap_err();
    assert!(
        error.contains("references unknown clause ID CLAUSE-MISSING"),
        "{error}"
    );

    let mut choice = source_value();
    choice["applicability"][0]["expression"] = json!({
        "kind": "implementation_defined",
        "choice_id": "IZ999"
    });
    let error = parse_value(&choice).unwrap_err();
    assert!(
        error.contains("references unknown implementation-defined ID IZ999"),
        "{error}"
    );
}

#[test]
fn implication_and_applicability_cycles_fail() {
    let mut implications = source_value();
    implications["implications"] = json!([
        {"id":"IMP-A","source":"G002","target":"G003","clause_anchors":[],"evidence":[]},
        {"id":"IMP-B","source":"G003","target":"G002","clause_anchors":[],"evidence":[]}
    ]);
    let error = parse_value(&implications).unwrap_err();
    assert!(
        error.contains("implication cycle: G002 -> G003 -> G002"),
        "{error}"
    );

    let mut applicability = source_value();
    applicability["applicability"] = json!([
        {"id":"APP-A","expression":{"kind":"applicability","applicability_id":"APP-B"}},
        {"id":"APP-ALWAYS","expression":{"kind":"always"}},
        {"id":"APP-B","expression":{"kind":"applicability","applicability_id":"APP-A"}}
    ]);
    let error = parse_value(&applicability).unwrap_err();
    assert!(
        error.contains("applicability cycle: APP-A -> APP-B -> APP-A"),
        "{error}"
    );
}

#[test]
fn applicability_depth_is_bounded() {
    let mut value = source_value();
    let mut expression = json!({"kind":"always"});
    for _ in 0..66 {
        expression = json!({"kind":"not","item":expression});
    }
    value["applicability"][0]["expression"] = expression;
    let error = parse_value(&value).unwrap_err();
    assert!(error.contains("exceeds applicability depth 64"), "{error}");
}

#[test]
fn canonical_hash_and_outputs_ignore_semantic_reordering() {
    let mut left = source_value();
    left["applicability"][0]["expression"] = json!({
        "kind":"all",
        "items":[
            {"kind":"feature","feature_id":"G002"},
            {"kind":"feature","feature_id":"G003"}
        ]
    });
    let mut right = left.clone();
    right["features"]
        .as_array_mut()
        .expect("features")
        .reverse();
    right["implementation_defined_choices"]
        .as_array_mut()
        .expect("choices")
        .reverse();
    right["applicability"][0]["expression"]["items"]
        .as_array_mut()
        .expect("items")
        .reverse();

    let left = parse_value(&left).expect("left validates");
    let right = parse_value(&right).expect("right validates");
    assert_eq!(left.canonical_json(), right.canonical_json());
    assert_eq!(left.hash(), right.hash());
    assert_eq!(
        render_outputs(&left).unwrap(),
        render_outputs(&right).unwrap()
    );
}

#[test]
fn repeated_generation_is_byte_identical() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    let source_path = root.join("spec/gql-profile/profile.json");
    std::fs::create_dir_all(source_path.parent().expect("source parent")).unwrap();
    std::fs::write(&source_path, SOURCE).unwrap();

    write_repository(root).expect("first generation");
    let profile = parse_profile(SOURCE).expect("source validates");
    let first = render_outputs(&profile)
        .unwrap()
        .into_iter()
        .map(|(path, _)| {
            let bytes = std::fs::read(root.join(&path)).unwrap();
            (path, bytes)
        })
        .collect::<BTreeMap<PathBuf, Vec<u8>>>();
    write_repository(root).expect("second generation");
    let second = first
        .keys()
        .map(|path| (path.clone(), std::fs::read(root.join(path)).unwrap()))
        .collect::<BTreeMap<PathBuf, Vec<u8>>>();
    assert_eq!(first, second);
}

#[test]
fn markdown_generation_escapes_table_content() {
    let mut value = source_value();
    let feature = value["features"]
        .as_array_mut()
        .expect("features")
        .iter_mut()
        .find(|item| item["id"] == "GC02")
        .expect("GC02 is present");
    feature["unsupported_rationale"] = json!("use <name> & left | right");

    let profile = parse_value(&value).expect("fixture validates");
    let markdown = render_outputs(&profile)
        .expect("outputs render")
        .into_iter()
        .find(|(path, _)| path == std::path::Path::new("spec/gql-profile/registry.md"))
        .expect("Markdown output")
        .1;
    assert!(markdown.contains("use &lt;name&gt; &amp; left \\| right"));
}

#[test]
fn checked_in_outputs_are_fresh() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    check_repository(&root).expect("run the documented --write command");
}

#[test]
fn schema_closes_every_object_rule() {
    fn visit(value: &Value, path: &str) {
        match value {
            Value::Object(object) => {
                if object.get("type") == Some(&Value::String("object".to_owned())) {
                    assert_eq!(
                        object.get("additionalProperties"),
                        Some(&Value::Bool(false)),
                        "open object schema at {path}"
                    );
                    let required = object["required"]
                        .as_array()
                        .expect("object schema declares required fields")
                        .iter()
                        .map(|item| item.as_str().expect("required field is text"))
                        .collect::<std::collections::BTreeSet<_>>();
                    let properties = object["properties"]
                        .as_object()
                        .expect("object schema declares properties")
                        .keys()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>();
                    assert_eq!(
                        required, properties,
                        "optional or undeclared field at {path}"
                    );
                }
                for (key, child) in object {
                    visit(child, &format!("{path}/{key}"));
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    visit(child, &format!("{path}/{index}"));
                }
            }
            _ => {}
        }
    }

    let schema: Value = serde_json::from_str(SCHEMA).expect("schema is JSON");
    visit(&schema, "#");
    assert_eq!(
        schema["$defs"]["extension_id"]["pattern"],
        "^IM_[A-Z0-9_]+$"
    );
}
