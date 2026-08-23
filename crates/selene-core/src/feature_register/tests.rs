use std::collections::HashSet;

use super::*;

#[test]
fn supported_features_has_no_duplicates() {
    // A duplicate would silently inflate the runtime inventory count.
    let mut seen = HashSet::new();
    for feature in SUPPORTED_FEATURES {
        assert!(
            seen.insert(*feature),
            "{feature} appears more than once in SUPPORTED_FEATURES"
        );
    }
}

#[test]
fn not_supported_rationale_has_no_duplicates() {
    let mut seen = HashSet::new();
    for (feature, _) in NOT_SUPPORTED_RATIONALE {
        assert!(
            seen.insert(*feature),
            "{feature} appears more than once in NOT_SUPPORTED_RATIONALE"
        );
    }
}

#[test]
fn supported_and_not_supported_are_disjoint() {
    // A feature cannot be both runtime-supported and assigned a rejection.
    let supported: HashSet<FeatureId> = SUPPORTED_FEATURES.iter().copied().collect();
    for (feature, _) in NOT_SUPPORTED_RATIONALE {
        assert!(
            !supported.contains(feature),
            "{feature} is in BOTH SUPPORTED_FEATURES and NOT_SUPPORTED_RATIONALE"
        );
    }
}

#[test]
fn ge08_is_reference_parameters_referenced_only() {
    // CONFORMANCE-00: GE08 is ISO Annex D Table D.1 row 77 / §17.7
    // "Reference parameters" — NOT a CAST feature. It must carry the correct
    // ISO name, must not be runtime-supported (reference parameters are unimplemented),
    // and — having no syntactic surface to reject — must NOT be in
    // NOT_SUPPORTED_RATIONALE (which is reserved for parser-rejected
    // features). It surfaces as the "referenced" status instead.
    assert_eq!(name_of(FeatureId::GE08), Some("Reference parameters"));
    assert!(
        !is_supported(FeatureId::GE08),
        "GE08 (Reference parameters) is not implemented and must not be runtime-supported"
    );
    assert!(
        non_supported_rationale(FeatureId::GE08).is_none(),
        "GE08 has no parser surface to reject; it is referenced-only, not rationalized"
    );
}

#[test]
fn ga05_cast_specification_is_supported() {
    // CONFORMANCE-00 (Codex review follow-up): GA05 "Cast specification"
    // (Annex D row 53 / §20.8) is the real ISO feature for CAST. Per ISO
    // Annex A item 52, without GA05 "conforming GQL language shall not
    // contain a <cast specification>" — CAST is gated behind GA05, not
    // baseline. The compatibility inventory reports the implemented cast
    // construct as runtime-supported, separately from formal claim state.
    assert_eq!(name_of(FeatureId::GA05), Some("Cast specification"));
    assert!(
        is_supported(FeatureId::GA05),
        "GA05 is runtime-supported: selene-db implements <cast specification>"
    );
    assert!(
        non_supported_rationale(FeatureId::GA05).is_none(),
        "GA05 is supported, so it has no non-supported rationale"
    );
}

#[test]
fn ga06_value_type_predicate_is_supported() {
    // CONFORMANCE-00 follow-up: GA06 "Value type predicate" (Annex A item 53 /
    // ISO §19.6) is the construct-level feature for `IS [NOT] TYPED <value
    // type>`. The compatibility inventory reports the implemented predicate as
    // runtime-supported, separately from formal claim state.
    assert_eq!(name_of(FeatureId::GA06), Some("Value type predicate"));
    assert!(
        is_supported(FeatureId::GA06),
        "GA06 is runtime-supported: selene-db implements <value type predicate>"
    );
    assert!(
        non_supported_rationale(FeatureId::GA06).is_none(),
        "GA06 is supported, so it has no non-supported rationale"
    );
}

#[test]
fn implication_conflicts_are_withdrawn_from_runtime_support() {
    for feature in [
        FeatureId::GC03,
        FeatureId::GE04,
        FeatureId::GE05,
        FeatureId::GH02,
        FeatureId::GG01,
        FeatureId::GG02,
        FeatureId::GG20,
        FeatureId::GG21,
        FeatureId::GS04,
        FeatureId::GV66,
        FeatureId::GV67,
    ] {
        assert!(
            !is_supported(feature),
            "{feature} must not be runtime-supported"
        );
        assert!(
            non_supported_rationale(feature).is_some(),
            "{feature} must explain the compatibility withdrawal"
        );
        assert!(
            is_flagger_accepted(feature),
            "{feature} syntax must remain parser-visible until M01-PR04"
        );
    }
}

#[test]
fn every_referenced_feature_resolves_by_string() {
    // Round-trips the stable string ABI both ways.
    for (feature, _) in REFERENCED_FEATURES {
        assert_eq!(feature_id_from_str(feature.as_str()), Some(*feature));
        assert!(name_of(*feature).is_some(), "{feature} has no display name");
    }
}

#[test]
fn annex_b_register_carries_no_pack_or_spec_05_residue() {
    // CORE-01: post-#196 the procedure-pack model is gone. No Annex B entry
    // may still name "pack" or point at the deleted spec 05 / spec 15.
    for record in ANNEX_B_REGISTER.iter() {
        let text = format!("{} {:?}", record.topic, record.decision).to_ascii_lowercase();
        assert!(
            !text.contains("pack"),
            "Annex B {} still references a pack",
            record.id.as_str()
        );
        assert!(
            !text.contains("spec 05") && !text.contains("spec 15"),
            "Annex B {} still points at a deleted spec",
            record.id.as_str()
        );
    }
}

#[test]
fn annex_b_register_has_no_duplicate_ids() {
    let mut seen = HashSet::new();
    for record in ANNEX_B_REGISTER.iter() {
        assert!(
            seen.insert(record.id),
            "{} appears more than once in ANNEX_B_REGISTER",
            record.id.as_str()
        );
    }
}
