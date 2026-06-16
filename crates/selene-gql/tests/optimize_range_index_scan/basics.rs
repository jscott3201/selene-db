use super::*;

#[test]
fn rewrites_greater_than_to_typed_index_range() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age > 30 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            bounds: TypedIndexBounds::GreaterThan(_),
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn combines_lower_and_upper_range_bounds() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age >= 30 AND n.age < 60 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            bounds: TypedIndexBounds::Range {
                lo_inclusive: true,
                hi_inclusive: false,
                ..
            },
            ..
        }
    ));
}

#[test]
fn temporal_literals_fire_typed_index_ranges() {
    let catalog = event_catalog();
    let plan = optimized_one(
        "MATCH (n:Event) WHERE n.event_date >= DATE '2026-05-01' AND n.event_date < DATE '2026-06-01' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::Date,
            bounds: TypedIndexBounds::Range { .. },
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());

    let plan = optimized_one(
        "MATCH (n:Event) WHERE n.started_at = LOCAL DATETIME '2026-05-07T12:34:56' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::LocalDateTime,
            bounds: TypedIndexBounds::Equality(_),
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());

    let plan = optimized_one(
        "MATCH (n:Event) WHERE n.occurred_at >= ZONED DATETIME '2026-05-07T12:00:00-04:00' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::ZonedDateTime,
            bounds: TypedIndexBounds::GreaterEqual(_),
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());

    let plan = optimized_one(
        "MATCH (n:Event) WHERE n.wall_time = LOCAL TIME '12:34:56' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::LocalTime,
            bounds: TypedIndexBounds::Equality(_),
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());

    let plan = optimized_one(
        "MATCH (n:Event) WHERE n.clock_time = TIME '12:34:56-04:00' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::ZonedTime,
            bounds: TypedIndexBounds::Equality(_),
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());

    let plan = optimized_one(
        "MATCH (n:Event) WHERE n.elapsed >= DURATION 'PT1H' AND n.elapsed < DURATION 'PT3H' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::Duration,
            bounds: TypedIndexBounds::Range { .. },
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn leaves_type_mismatch_unchanged() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age = 'old' RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::Linear));
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn rewrites_scan_under_path_search_selector() {
    let plan = optimized_one("MATCH ANY (n:Person {age: 30}) RETURN n", &person_catalog());
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::TypedIndexRange { .. }));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn sentinel_range_index_scan_snapshot() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age >= 30 AND n.age < 60 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    let summary = match scan.access {
        ScanAccess::TypedIndexRange {
            bounds:
                TypedIndexBounds::Range {
                    lo_inclusive,
                    hi_inclusive,
                    ..
                },
            ..
        } => format!(
            "access=range\nlo_inclusive={lo_inclusive}\nhi_inclusive={hi_inclusive}\nresidual_filters={}",
            scan.property_predicates.len()
        ),
        ref other => format!("unexpected={other:?}"),
    };

    insta::assert_snapshot!(summary, @r###"
access=range
lo_inclusive=true
hi_inclusive=false
residual_filters=0
"###);
}
