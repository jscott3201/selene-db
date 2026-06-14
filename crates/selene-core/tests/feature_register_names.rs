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
