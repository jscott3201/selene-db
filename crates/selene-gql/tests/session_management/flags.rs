use super::*;

#[test]
fn flagger_stamps_set_value_gs03() {
    assert!(walked_features("SESSION SET VALUE $p = 1").contains(&FeatureId::GS03));
}

#[test]
fn flagger_stamps_set_value_declared_type_features() {
    let observed = walked_features("SESSION SET VALUE $p INT8 = 1");

    assert!(observed.contains(&FeatureId::GS03));
    assert!(
        observed.contains(&FeatureId::GV02) && observed.contains(&FeatureId::GV09),
        "INT8 target type must stamp GV02/GV09, observed {observed:?}"
    );
}

#[test]
fn flagger_walks_set_value_initializer_features() {
    let observed = walked_features("SESSION SET VALUE $p = $base :: INT8");

    assert!(observed.contains(&FeatureId::GS03));
    assert!(observed.contains(&FeatureId::GE04));
    assert!(observed.contains(&FeatureId::GE05));
    assert!(observed.contains(&FeatureId::IM_TYPED_PARAMS));
    assert!(
        observed.contains(&FeatureId::GV02) && observed.contains(&FeatureId::GV09),
        "typed RHS parameter must stamp INT8 features, observed {observed:?}"
    );
}

#[test]
fn flagger_stamps_set_time_zone_gs15() {
    assert!(walked_features("SESSION SET TIME ZONE '+00:00'").contains(&FeatureId::GS15));
}

#[test]
fn flagger_stamps_reset_targets() {
    assert!(walked_features("SESSION RESET").contains(&FeatureId::GS04));
    assert!(walked_features("SESSION RESET SCHEMA").contains(&FeatureId::GS05));
    assert!(walked_features("SESSION RESET GRAPH").contains(&FeatureId::GS06));
    assert!(walked_features("SESSION RESET PARAMETERS").contains(&FeatureId::GS08));
    assert!(walked_features("SESSION RESET TIME ZONE").contains(&FeatureId::GS07));
    assert!(walked_features("SESSION RESET PARAMETER $p").contains(&FeatureId::GS16));
}

#[test]
fn flagger_reset_parameter_co_stamps_gs08_and_gs16() {
    // ISO/IEC 39075:2024 section 7.2: `SESSION RESET PARAMETER <name>` uses both
    // the RESET-PARAMETER surface (CR6 → GS08) and the parameter-name argument
    // (CR7 → GS16); a faithful Flagger stamps both. `RESET ALL PARAMETERS` is
    // the GS08 surface without the parameter-name argument, so it stamps GS08
    // only (no GS16).
    let single = walked_features("SESSION RESET PARAMETER $p");
    assert!(
        single.contains(&FeatureId::GS08),
        "RESET PARAMETER implies GS08"
    );
    assert!(
        single.contains(&FeatureId::GS16),
        "RESET PARAMETER implies GS16"
    );

    let all = walked_features("SESSION RESET PARAMETERS");
    assert!(all.contains(&FeatureId::GS08));
    assert!(
        !all.contains(&FeatureId::GS16),
        "RESET ALL PARAMETERS has no parameter-name argument, so no GS16"
    );
}

#[test]
fn flagger_does_not_stamp_session_close() {
    // SESSION CLOSE (section 7.3) has no ISO feature code.
    assert!(walked_features("SESSION CLOSE").is_empty());
}

// ---------------------------------------------------------------------------
// Deferred D1-blocked forms (GS01 / GS02)
// ---------------------------------------------------------------------------
