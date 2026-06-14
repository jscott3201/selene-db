//! Public feature display-name pins for Annex D alignment.

use selene_core::feature_register::{FeatureId, name_of};

#[test]
fn procedure_feature_names_match_annex_d() {
    for (feature, expected_name) in [
        (FeatureId::GP01, "Inline procedure"),
        (
            FeatureId::GP02,
            "Inline procedure with implicit nested variable scope",
        ),
        (
            FeatureId::GP03,
            "Inline procedure with explicit nested variable scope",
        ),
        (
            FeatureId::GP06,
            "Procedure-local value variable definitions: value variables based on simple expressions",
        ),
        (
            FeatureId::GP07,
            "Procedure-local value variable definitions: value variable based on subqueries",
        ),
        (
            FeatureId::GP09,
            "Procedure-local binding table variable definitions: binding table variables based on simple expressions or references",
        ),
        (
            FeatureId::GP10,
            "Procedure-local binding table variable definitions: binding table variables based on subqueries",
        ),
        (
            FeatureId::GP12,
            "Procedure-local graph variable definitions: graph variables based on simple expressions or references",
        ),
        (
            FeatureId::GP13,
            "Procedure-local graph variable definitions: graph variables based on subqueries",
        ),
        (FeatureId::GP18, "Catalog and data statement mixing"),
    ] {
        assert_eq!(name_of(feature), Some(expected_name));
    }
}

#[test]
fn core_language_feature_names_match_annex_d() {
    for (feature, expected_name) in [
        (FeatureId::G002, "Different-edges match mode"),
        (FeatureId::G003, "Explicit REPEATABLE ELEMENTS keyword"),
        (FeatureId::G036, "Quantified edges"),
        (FeatureId::G037, "Questioned paths"),
        (FeatureId::G060, "Bounded graph pattern quantifiers"),
        (FeatureId::G061, "Unbounded graph pattern quantifiers"),
        (FeatureId::GE04, "Graph parameters"),
        (FeatureId::GE05, "Binding table parameters"),
        (FeatureId::GE07, "Boolean XOR"),
        (FeatureId::GT03, "Use of multiple graphs in a transaction"),
    ] {
        assert_eq!(name_of(feature), Some(expected_name));
    }
}

#[test]
fn session_feature_names_match_annex_d() {
    for (feature, expected_name) in [
        (
            FeatureId::GS10,
            "SESSION SET command: session-local binding table parameters based on subqueries",
        ),
        (
            FeatureId::GS11,
            "SESSION SET command: session-local value parameters based on subqueries",
        ),
        (
            FeatureId::GS12,
            "SESSION SET command: session-local graph parameters based on simple graph expressions or references",
        ),
        (
            FeatureId::GS13,
            "SESSION SET command: session-local binding table parameters based on simple expressions or references",
        ),
        (
            FeatureId::GS14,
            "SESSION SET command: session-local value parameters based on simple expressions",
        ),
    ] {
        assert_eq!(name_of(feature), Some(expected_name));
    }
}
