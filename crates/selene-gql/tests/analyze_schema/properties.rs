use super::*;

#[test]
fn rejects_undeclared_property_on_insert_and_set() {
    let graph_type = person_company_graph_type();
    let insert_error = schema_error("INSERT (n:Person { name: 'Alice', age: 42 })", &graph_type);
    assert!(matches!(
        insert_error,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));

    let set_error = schema_error("MATCH (n:Person) SET n.age = 42", &graph_type);
    assert!(matches!(
        set_error,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));
}

#[test]
fn match_then_match_with_labels_uses_refined_labels_for_set_validation() {
    let graph_type = person_company_graph_type();
    let error = schema_error("MATCH (n) MATCH (n:Person) SET n.foo = 1", &graph_type);
    assert!(matches!(
        error,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));
}

#[test]
fn rejects_property_type_mismatch_on_insert_and_set() {
    let graph_type = person_company_graph_type();
    let insert_error = schema_error("INSERT (n:Person { name: 42 })", &graph_type);
    assert!(matches!(
        insert_error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));

    let set_error = schema_error("MATCH (n:Person) SET n.name = 42", &graph_type);
    assert!(matches!(
        set_error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));
}

#[test]
fn accepts_record_literal_write_into_record_property() {
    // Regression pin (analyzer/runtime divergence): L1c-a binds a record literal to the
    // OPEN record type, and L1c-d lowers every RECORD property declaration to the
    // `RecordTyped` tag — so a record write into a typed RECORD property must pass the
    // coarse static schema gate (closed-field conformance is enforced at commit time →
    // G2000). Before the `property_type_compatible` fix, the analyzer rejected the only
    // value form the `RECORD{..}` constructor can produce, making typed-RECORD properties
    // impossible to populate through GQL.
    let graph_type = host_record_graph_type();

    analyze_with_schema(
        "INSERT (n:Host {config: RECORD{host: 'h', port: 1}})",
        &graph_type,
    )
    .expect("record literal INSERT into a RECORD property analyzes");

    analyze_with_schema(
        "MATCH (n:Host) SET n.config = RECORD{host: 'h', port: 1}",
        &graph_type,
    )
    .expect("record literal SET into a RECORD property analyzes");

    // The gate is not broken open: a record literal written to a scalar (`String`)
    // property is still a static type mismatch.
    let mismatch = schema_error(
        "INSERT (n:Person {name: RECORD{x: 1}})",
        &person_company_graph_type(),
    );
    assert!(matches!(
        mismatch,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));
}

#[test]
fn rejects_duplicate_set_map_keys_with_mismatched_values() {
    let graph_type = person_company_graph_type();
    let error = schema_error(
        "MATCH (n:Person) SET n = { name: 'A', name: 1 }",
        &graph_type,
    );
    assert!(matches!(
        error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));
}

#[test]
fn rejects_missing_and_removed_required_property() {
    let graph_type = person_company_graph_type();
    let missing = schema_error("INSERT (n:Person)", &graph_type);
    assert!(matches!(
        missing,
        AnalysisError::SchemaRequiredPropertyMissing { .. }
    ));

    let removed = schema_error("MATCH (n:Person) REMOVE n.name", &graph_type);
    assert!(matches!(
        removed,
        AnalysisError::SchemaRequiredPropertyRemoved { .. }
    ));
}

#[test]
fn rejects_invalid_insert_label_forms() {
    let graph_type = person_company_graph_type();
    for source in [
        "INSERT (n { name: 'Alice' })",
        "INSERT (n:Person|Company { name: 'Alice' })",
        "INSERT (n:!Person { name: 'Alice' })",
        "INSERT (n:% { name: 'Alice' })",
    ] {
        let error = schema_error(source, &graph_type);
        assert!(matches!(
            error,
            AnalysisError::SchemaInvalidInsertLabelExpr { .. }
        ));
    }
}

#[test]
fn required_insert_property_can_be_supplied_by_later_set() {
    let graph_type = person_only_graph_type();
    analyze_with_schema("INSERT (n:Person) SET n.name = 'Alice'", &graph_type)
        .expect("later SET property satisfies required property");
    analyze_with_schema("INSERT (n:Person) SET n = { name: 'Alice' }", &graph_type)
        .expect("later SET property map satisfies required property");

    let error = schema_error("INSERT (n:Person) SET n :Active", &graph_type);
    assert!(matches!(
        error,
        AnalysisError::SchemaRequiredPropertyMissing { .. }
    ));
}

#[test]
fn analyzer_resolves_schema_properties_by_binding_id() {
    let graph_type = person_only_graph_type();
    analyze_with_schema("INSERT (n:Person) SET n.name = 'Alice'", &graph_type)
        .expect("later SET target satisfies required property through its BindingId");
}

#[test]
fn analyzer_resolves_schema_labels_by_binding_id() {
    let graph_type = person_company_graph_type();
    analyze_with_schema(
        concat!(
            "INSERT (n:Person { name: 'A' }) ",
            "INSERT (n)-[:WORKS_AT]->(b:Company { name: 'B' })"
        ),
        &graph_type,
    )
    .expect("reused INSERT endpoint resolves through the original BindingId");
}

#[test]
fn validates_static_candidate_sets_when_candidates_agree() {
    let graph_type = person_company_graph_type();
    analyze_with_schema("MATCH (n:Person) SET n.name = 'Alice'", &graph_type)
        .expect("all Person candidates agree on name");
    analyze_with_schema("MATCH (n:Person) REMOVE n.nickname", &graph_type)
        .expect("all Person candidates agree nickname is optional");
}

#[test]
fn validates_resolved_json_property_writes() {
    let graph_type = json_graph_type();
    analyze_with_schema(
        r#"INSERT (:Thing {payload: CAST('{"a":1}' AS JSON)})"#,
        &graph_type,
    )
    .expect("resolved JSON insert satisfies JSON property");
    analyze_with_schema(
        r#"MATCH (n:Thing) SET n.payload = CAST('{"a":2}' AS JSON)"#,
        &graph_type,
    )
    .expect("resolved JSON SET satisfies JSON property");

    let mismatch = schema_error(
        r#"MATCH (n:Thing) SET n.payload = CAST('{"a":2}' AS STRING)"#,
        &graph_type,
    );
    assert!(matches!(
        mismatch,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));
    let removed = schema_error("MATCH (n:Thing) REMOVE n.payload", &graph_type);
    assert!(matches!(
        removed,
        AnalysisError::SchemaRequiredPropertyRemoved { .. }
    ));
}

#[test]
fn defers_static_candidate_sets_when_candidates_disagree() {
    let graph_type = ambiguous_property_graph_type();
    analyze_with_schema("MATCH (n:Person) SET n.flag = 1", &graph_type)
        .expect("candidate disagreement defers to runtime");
}
