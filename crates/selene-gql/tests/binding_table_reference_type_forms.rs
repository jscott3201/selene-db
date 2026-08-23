//! ISO binding-table reference value type form coverage.

use selene_core::feature_register::FeatureId;
use selene_gql::{GqlStatus, ParserError, parse};

#[test]
fn binding_table_reference_type_forms_report_gv61_unsupported() {
    for source in [
        "RETURN NULL IS TYPED TABLE {} AS ok",
        "RETURN NULL IS TYPED TABLE { id :: INT } AS ok",
        "RETURN NULL IS TYPED TABLE { id TYPED INT } AS ok",
        "RETURN NULL IS TYPED TABLE { id INT } AS ok",
        "RETURN NULL IS TYPED BINDING TABLE { id :: INT, label :: STRING } AS ok",
        "RETURN NULL IS TYPED TABLE { id :: INT } NOT NULL AS ok",
    ] {
        let err = parse(source).expect_err("TABLE reference type remains runtime-unsupported");
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("{source} should report unsupported GV61, got {err:?}");
        };
        assert_eq!(feature_id, FeatureId::GV61, "{source}");
    }
}

#[test]
fn binding_table_reference_type_rejects_missing_field_specification() {
    for source in [
        "RETURN NULL IS TYPED TABLE AS ok",
        "RETURN NULL IS TYPED BINDING TABLE AS ok",
    ] {
        let err = parse(source).expect_err("TABLE without fields is not an ISO binding table type");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn binding_table_reference_type_rejects_duplicate_field_names() {
    let source = "RETURN NULL IS TYPED TABLE { id INT, id TYPED STRING } AS ok";
    let err = parse(source).expect_err("duplicate table columns reject before feature gate");
    assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR);
    assert!(
        err.to_string()
            .contains("duplicate binding table field type name: id"),
        "{err:?}"
    );
}

#[test]
fn binding_table_reference_type_requires_token_boundary() {
    for source in [
        "RETURN NULL IS TYPED TABLEX {} AS ok",
        "RETURN NULL IS TYPED BINDINGTABLE {} AS ok",
    ] {
        let err = parse(source).expect_err("run-together table keywords reject");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}
