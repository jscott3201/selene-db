//! Annex B inventory, decision, applicability, and generated-surface tests.

use std::collections::BTreeMap;

use selene_profile::{
    ANNEX_B_CATEGORY_COUNTS, ANNEX_B_IA, ANNEX_B_ID, ANNEX_B_IE, ANNEX_B_IL, ANNEX_B_IS,
    ANNEX_B_IV, ANNEX_B_IW, ANNEX_B_LOOKUP_TEST_VECTORS, AnnexBDecision, AnnexBValue,
    annex_b_by_id, annex_b_records, parse_profile, render_outputs,
};
use serde_json::{Value, json};

const SOURCE: &str = include_str!("../../../spec/gql-profile/profile.json");

const EXPECTED: &[(&str, &[&str])] = &[
    (
        "IA",
        &[
            "IA001", "IA002", "IA003", "IA004", "IA005", "IA006", "IA007", "IA010", "IA011",
            "IA012", "IA013", "IA014", "IA015", "IA016", "IA017", "IA019", "IA020", "IA021",
            "IA023", "IA025", "IA026",
        ],
    ),
    (
        "ID",
        &[
            "ID001", "ID002", "ID003", "ID004", "ID005", "ID006", "ID016", "ID017", "ID022",
            "ID023", "ID028", "ID034", "ID037", "ID048", "ID049", "ID057", "ID058", "ID059",
            "ID061", "ID062", "ID063", "ID064", "ID065", "ID066", "ID067", "ID068", "ID069",
            "ID070", "ID074", "ID075", "ID076", "ID079", "ID085", "ID086", "ID089", "ID090",
            "ID091", "ID095", "ID096", "ID097", "ID098", "ID099",
        ],
    ),
    (
        "IE",
        &[
            "IE001", "IE002", "IE003", "IE004", "IE005", "IE006", "IE007", "IE008", "IE009",
            "IE010",
        ],
    ),
    (
        "IL",
        &[
            "IL001", "IL002", "IL003", "IL009", "IL010", "IL011", "IL013", "IL015", "IL018",
            "IL020", "IL023", "IL024",
        ],
    ),
    ("IS", &["IS001"]),
    (
        "IV",
        &[
            "IV001", "IV002", "IV003", "IV008", "IV010", "IV011", "IV012", "IV014", "IV015",
            "IV016", "IV023",
        ],
    ),
    (
        "IW",
        &[
            "IW001", "IW002", "IW003", "IW004", "IW005", "IW006", "IW007", "IW010", "IW011",
            "IW012", "IW014", "IW015", "IW016", "IW017", "IW018", "IW019", "IW021", "IW022",
            "IW023", "IW025",
        ],
    ),
];

fn source_value() -> Value {
    serde_json::from_str(SOURCE).expect("checked-in profile is JSON")
}

fn parse_value(value: &Value) -> Result<selene_profile::ValidatedProfile, String> {
    parse_profile(&serde_json::to_string(value).expect("fixture serializes"))
        .map_err(|error| error.to_string())
}

fn record_mut<'a>(value: &'a mut Value, id: &str) -> &'a mut Value {
    value["implementation_defined_choices"]
        .as_array_mut()
        .expect("Annex B records")
        .iter_mut()
        .find(|record| record["id"] == id)
        .expect("known Annex B ID")
}

#[test]
fn exact_inventory_categories_register_and_lookup_are_complete() {
    let generated = [
        ANNEX_B_IA, ANNEX_B_ID, ANNEX_B_IE, ANNEX_B_IL, ANNEX_B_IS, ANNEX_B_IV, ANNEX_B_IW,
    ];
    let expected_counts = EXPECTED
        .iter()
        .map(|(category, ids)| (*category, ids.len()))
        .collect::<Vec<_>>();
    assert_eq!(ANNEX_B_CATEGORY_COUNTS, expected_counts);
    assert_eq!(annex_b_records().count(), 117);
    assert_eq!(ANNEX_B_LOOKUP_TEST_VECTORS.len(), 117);

    for ((category, expected), records) in EXPECTED.iter().zip(generated) {
        assert_eq!(records.len(), expected.len(), "{category} count");
        assert_eq!(
            records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            *expected
        );
    }
    for (expected_order, (id, vector_order)) in ANNEX_B_LOOKUP_TEST_VECTORS.iter().enumerate() {
        assert_eq!(*vector_order, expected_order);
        assert_eq!(
            annex_b_by_id(id.as_str()).map(|record| record.id),
            Some(*id)
        );
    }
    assert!(annex_b_by_id("IV001-IV016").is_none());
    assert!(annex_b_by_id("IA999").is_none());
}

#[test]
fn malformed_range_duplicate_missing_and_extra_ids_fail() {
    let mut range = source_value();
    record_mut(&mut range, "IA001")["id"] = json!("IA001-IA002");
    let error = parse_value(&range).unwrap_err();
    assert!(
        error.contains("malformed implementation-defined ID"),
        "{error}"
    );

    let mut duplicate = source_value();
    let first = duplicate["implementation_defined_choices"][0].clone();
    duplicate["implementation_defined_choices"]
        .as_array_mut()
        .unwrap()
        .push(first);
    let error = parse_value(&duplicate).unwrap_err();
    assert!(
        error.contains("duplicate implementation-defined choice ID IA001"),
        "{error}"
    );

    let mut missing = source_value();
    missing["implementation_defined_choices"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let error = parse_value(&missing).unwrap_err();
    assert!(error.contains("missing [IA001]"), "{error}");

    let mut extra = source_value();
    record_mut(&mut extra, "IA001")["id"] = json!("IA999");
    let error = parse_value(&extra).unwrap_err();
    assert!(error.contains("missing [IA001]; extra [IA999]"), "{error}");
}

#[test]
fn unknown_clause_evidence_applicability_and_owner_fail() {
    for (field, replacement, expected) in [
        (
            "clause_anchors",
            json!(["CLAUSE-MISSING"]),
            "unknown clause ID CLAUSE-MISSING",
        ),
        (
            "evidence",
            json!(["EVID-MISSING"]),
            "unknown evidence ID EVID-MISSING",
        ),
        (
            "applicability",
            json!("APP-MISSING"),
            "unknown applicability ID APP-MISSING",
        ),
    ] {
        let mut value = source_value();
        record_mut(&mut value, "IA001")[field] = replacement;
        let error = parse_value(&value).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }

    let mut owner = source_value();
    record_mut(&mut owner, "IA002")["decision"]["owner"] = json!("M99-PR99");
    let error = parse_value(&owner).unwrap_err();
    assert!(error.contains("not a known bounded work item"), "{error}");
}

#[test]
fn closed_decision_and_value_decode_rejects_missing_extra_and_incompatible_fields() {
    let mut missing = source_value();
    record_mut(&mut missing, "IA001")["decision"]
        .as_object_mut()
        .unwrap()
        .remove("value");
    let error = parse_value(&missing).unwrap_err();
    assert!(error.contains("missing field `value`"), "{error}");

    let mut extra = source_value();
    record_mut(&mut extra, "IA002")["decision"]["value"] = json!({"type":"boolean","value":true});
    let error = parse_value(&extra).unwrap_err();
    assert!(error.contains("unknown field `value`"), "{error}");

    let mut incompatible = source_value();
    record_mut(&mut incompatible, "IA001")["decision"]["value"]["value"] = json!("true");
    let error = parse_value(&incompatible).unwrap_err();
    assert!(error.contains("invalid type: string"), "{error}");

    let mut value_extra = source_value();
    record_mut(&mut value_extra, "IA001")["decision"]["value"]["extra"] = json!(false);
    let error = parse_value(&value_extra).unwrap_err();
    assert!(error.contains("unknown field `extra`"), "{error}");
}

#[test]
fn applicability_uses_closure_extensions_composition_references_and_negation() {
    let mut value = source_value();
    value["applicability"].as_array_mut().unwrap().extend([
        json!({"id":"APP-TEST-CLOSURE","expression":{"kind":"feature","feature_id":"GV65"}}),
        json!({"id":"APP-TEST-EXTENSION","expression":{"kind":"extension","extension_id":"IM_JSON"}}),
        json!({"id":"APP-TEST-COMPOSED","expression":{"kind":"all","items":[
            {"kind":"applicability","applicability_id":"APP-TEST-CLOSURE"},
            {"kind":"applicability","applicability_id":"APP-TEST-EXTENSION"},
            {"kind":"not","item":{"kind":"feature","feature_id":"GP18"}}
        ]}}),
    ]);
    let profile = parse_value(&value).expect("composed applicability validates");
    assert_eq!(profile.applicability("APP-TEST-CLOSURE"), Some(true));
    assert_eq!(profile.applicability("APP-TEST-EXTENSION"), Some(true));
    assert_eq!(profile.applicability("APP-TEST-COMPOSED"), Some(true));
    assert_eq!(profile.applicability("APP-MIXED-TRANSACTIONS"), Some(false));

    let mut unsupported_extension = source_value();
    let extension = unsupported_extension["implementation_extensions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|extension| extension["id"] == "IM_JSON")
        .unwrap();
    extension["runtime_support"] = json!("unsupported");
    extension["unsupported_rationale"] = json!("Synthetic unsupported extension fixture.");
    unsupported_extension["supported_feature_order"]
        .as_array_mut()
        .unwrap()
        .retain(|id| id != "IM_JSON");
    unsupported_extension["applicability"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"APP-TEST-EXTENSION-OFF","expression":{"kind":"extension","extension_id":"IM_JSON"}}));
    let profile = parse_value(&unsupported_extension).expect("unsupported extension validates");
    assert_eq!(profile.applicability("APP-TEST-EXTENSION-OFF"), Some(false));
}

#[test]
fn disposition_must_match_evaluated_applicability() {
    let mut false_selected = source_value();
    record_mut(&mut false_selected, "IE006")["decision"] = json!({
        "disposition":"selected",
        "value":{"type":"boolean","value":false},
        "rationale":"Fixture selection.",
        "stability":"stable",
        "visibility":"internal"
    });
    let error = parse_value(&false_selected).unwrap_err();
    assert!(
        error.contains("IE006 applicability is false but disposition is selected"),
        "{error}"
    );

    let mut true_not_applicable = source_value();
    record_mut(&mut true_not_applicable, "IA001")["decision"] =
        json!({"disposition":"not_applicable","reason":"Fixture mismatch."});
    let error = parse_value(&true_not_applicable).unwrap_err();
    assert!(
        error.contains("IA001 applicability is true but disposition is not_applicable"),
        "{error}"
    );
}

#[test]
fn release_claim_and_placeholder_and_size_guards_are_strict() {
    let mut release = source_value();
    release["release_claimable"] = json!(true);
    let error = parse_value(&release).unwrap_err();
    assert!(
        error.contains("release-claimable profile has pending decision IA002"),
        "{error}"
    );

    let mut placeholder = source_value();
    record_mut(&mut placeholder, "IA002")["decision"]["reason"] = json!("TODO later");
    let error = parse_value(&placeholder).unwrap_err();
    assert!(error.contains("contains placeholder text"), "{error}");

    let mut repeated = source_value();
    record_mut(&mut repeated, "IA001")["topic"] = json!("x".repeat(97));
    let error = parse_value(&repeated).unwrap_err();
    assert!(error.contains("topic exceeds 96 characters"), "{error}");

    let mut no_evidence = source_value();
    record_mut(&mut no_evidence, "IA001")["evidence"] = json!([]);
    let error = parse_value(&no_evidence).unwrap_err();
    assert!(
        error.contains("IA001 must cite at least one evidence"),
        "{error}"
    );
}

#[test]
fn checked_in_records_have_no_placeholder_text() {
    let lower = SOURCE.to_ascii_lowercase();
    for marker in ["\"tbd\"", "\"todo\"", "placeholder", "\"unknown\""] {
        assert!(
            !lower.contains(marker),
            "checked-in profile contains {marker}"
        );
    }

    let source = source_value();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for evidence in source["evidence"].as_array().unwrap() {
        let reference = evidence["reference"].as_str().unwrap();
        let path = reference.split('#').next().unwrap();
        assert!(
            root.join(path).is_file(),
            "{} references missing repository evidence {path}",
            evidence["id"]
        );
    }
}

#[test]
fn ia001_float_relocation_and_id086_are_truthful() {
    let ia001 = annex_b_by_id("IA001").unwrap();
    assert_eq!(ia001.topic, "Result declared-type exposure");
    assert_eq!(
        ia001.decision,
        AnnexBDecision::Selected {
            value: AnnexBValue::Boolean(true),
            rationale: "Row results expose analyzer-inferred column types through the public binding-table schema.",
            stability: selene_profile::RuntimeDecisionStability::Stable,
            visibility: selene_profile::RuntimeDecisionVisibility::Public,
        }
    );
    assert!(!format!("{ia001:?}").to_ascii_lowercase().contains("float"));

    let id037 = annex_b_by_id("ID037").unwrap();
    let AnnexBDecision::Selected {
        value: AnnexBValue::OrderedStringList(widths),
        ..
    } = id037.decision
    else {
        panic!("ID037 must carry the relocated float-width policy")
    };
    assert!(
        widths
            .iter()
            .any(|value| value.contains("binary precision 24"))
    );
    assert!(
        widths
            .iter()
            .any(|value| value.contains("binary precision 53"))
    );

    let id086 = annex_b_by_id("ID086").unwrap();
    assert!(id086.evidence.contains(&"EVID-MATCH-MODE"));
    assert!(matches!(
        id086.decision,
        AnnexBDecision::Selected {
            value: AnnexBValue::Identifier("REPEATABLE ELEMENTS"),
            ..
        }
    ));
}

#[test]
fn il010_matches_the_production_i64_literal_boundary() {
    let il010 = annex_b_by_id("IL010").expect("IL010 is registered");
    assert!(matches!(
        il010.decision,
        AnnexBDecision::Selected {
            value: AnnexBValue::UnsignedInteger(19),
            stability: selene_profile::RuntimeDecisionStability::Stable,
            visibility: selene_profile::RuntimeDecisionVisibility::Public,
            ..
        }
    ));
}

#[test]
fn generated_markdown_is_complete_and_stable() {
    let profile = parse_profile(SOURCE).expect("profile validates");
    let outputs = render_outputs(&profile).expect("outputs render");
    let report = outputs
        .iter()
        .find(|(path, _)| {
            path == std::path::Path::new("docs/gql/conformance/implementation-defined.md")
        })
        .map(|(_, contents)| contents)
        .expect("implementation-defined report");
    assert!(report.contains("not normative text and does not make a conformance claim"));
    for (_, ids) in EXPECTED {
        for id in *ids {
            assert_eq!(report.matches(&format!("| {id} |")).count(), 1, "{id}");
        }
    }

    let generated = outputs.into_iter().collect::<BTreeMap<_, _>>();
    for category in ["ia", "id", "ie", "il", "is", "iv", "iw"] {
        assert!(generated.contains_key(&std::path::PathBuf::from(format!(
            "crates/selene-profile/src/generated/annex_b_{category}.rs"
        ))));
    }
}
