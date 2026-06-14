use std::collections::HashSet;

use super::*;

#[test]
fn supported_features_has_no_duplicates() {
    // CORE-15: a duplicated feature in SUPPORTED would silently inflate the
    // conformance claim count and is otherwise invisible.
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
    // CORE-15: SUPPORTED ∩ NOT_SUPPORTED = ∅ — a feature cannot be both
    // claimed and rationalized-as-unclaimed.
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
    // ISO name, must NOT be claimed (reference parameters are unimplemented),
    // and — having no syntactic surface to reject — must NOT be in
    // NOT_SUPPORTED_RATIONALE (which is reserved for parser-rejected
    // features). It surfaces as the "referenced" status instead.
    assert_eq!(name_of(FeatureId::GE08), Some("Reference parameters"));
    assert!(
        !is_supported(FeatureId::GE08),
        "GE08 (Reference parameters) is not implemented and must not be claimed"
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
    // baseline. selene-db implements the cast construct, so it CLAIMS GA05
    // (and so GA05 carries no non-supported rationale).
    assert_eq!(name_of(FeatureId::GA05), Some("Cast specification"));
    assert!(
        is_supported(FeatureId::GA05),
        "GA05 is claimed: selene-db implements <cast specification>"
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
    // type>`. selene-db implements the typed predicate, so it CLAIMS GA06 and
    // carries no non-supported rationale.
    assert_eq!(name_of(FeatureId::GA06), Some("Value type predicate"));
    assert!(
        is_supported(FeatureId::GA06),
        "GA06 is claimed: selene-db implements <value type predicate>"
    );
    assert!(
        non_supported_rationale(FeatureId::GA06).is_none(),
        "GA06 is supported, so it has no non-supported rationale"
    );
}

#[test]
fn gg21_explicit_key_label_sets_is_claimed() {
    // 813: GG21 "Explicit element type key label sets" is now CLAIMED. The
    // type-DDL grammar parses the explicit `<...type key label set>` (the
    // `=>` <implies> marker, ISO §18.2/18.3) and the flagger stamps it. A
    // claimed feature carries no non-supported rationale.
    assert_eq!(
        name_of(FeatureId::GG21),
        Some("Explicit element type key label sets")
    );
    assert!(
        is_supported(FeatureId::GG21),
        "GG21 is claimed: the explicit key-label-set `=>` syntax is parsed and flagged"
    );
    assert!(
        non_supported_rationale(FeatureId::GG21).is_none(),
        "GG21 is supported, so it has no non-supported rationale"
    );
    // §24.7 implied-feature-relationships: GG21 implies GG02 ("Graph with a
    // closed graph type"). The implication is satisfied — GG02 is claimed —
    // so claiming GG21 is implication-consistent and not a phantom claim.
    assert!(
        is_supported(FeatureId::GG02) && is_supported(FeatureId::GG20),
        "GG21 implies GG02 (closed graph type); GG02 + GG20 stay claimed"
    );
}

#[test]
fn at_least_one_graph_type_feature_is_claimed() {
    // CORE-15: ISO §4 requires at least one of GG01 (open graph) or GG02
    // (closed graph). selene-db claims both, but the floor is the OR.
    assert!(
        is_supported(FeatureId::GG01) || is_supported(FeatureId::GG02),
        "neither GG01 (open graph) nor GG02 (closed graph) is claimed"
    );
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
    for (id, choice) in ANNEX_B_REGISTER {
        let haystacks = [choice.choice, choice.settled_in];
        for text in haystacks {
            let lower = text.to_ascii_lowercase();
            assert!(
                !lower.contains("pack"),
                "Annex B {} still references a 'pack': {text:?}",
                id.as_str()
            );
            assert!(
                !lower.contains("spec 05") && !lower.contains("spec 15"),
                "Annex B {} still points at a deleted spec: {text:?}",
                id.as_str()
            );
        }
    }
}

#[test]
fn annex_b_register_has_no_duplicate_ids() {
    let mut seen = HashSet::new();
    for (id, _) in ANNEX_B_REGISTER {
        assert!(
            seen.insert(*id),
            "{} appears more than once in ANNEX_B_REGISTER",
            id.as_str()
        );
    }
}
