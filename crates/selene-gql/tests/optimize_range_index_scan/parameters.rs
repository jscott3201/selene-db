use super::*;

#[test]
fn duplicate_lower_bounds_keep_the_tightest() {
    // `age > 10 AND age > 5` must produce `> 10`, never `> 5` — using the
    // weaker bound while removing both predicates would let rows with
    // 5 < age <= 10 leak through the index scan.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age > 10 AND n.age > 5 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    let TypedIndexBounds::GreaterThan(IndexKey::Literal(literal)) = bounds else {
        panic!("expected GreaterThan literal bound, got {bounds:?}");
    };
    assert!(
        matches!(literal, selene_gql::Literal::Integer(10, _)),
        "expected tightest lower bound 10, got {literal:?}"
    );
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn duplicate_upper_bounds_keep_the_tightest() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age < 50 AND n.age < 100 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    let TypedIndexBounds::LessThan(IndexKey::Literal(literal)) = bounds else {
        panic!("expected LessThan literal bound, got {bounds:?}");
    };
    assert!(matches!(literal, selene_gql::Literal::Integer(50, _)));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn parameterized_equality_fires_typed_index_range() {
    // BRIEF-154 bar 1: `WHERE n.col = $p` against a typed-indexed column plans
    // as `TypedIndexRange` with `Equality(IndexKey::Parameter)`, not Linear.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age = $threshold RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    let TypedIndexBounds::Equality(IndexKey::Parameter { name, .. }) = bounds else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "threshold");
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn parameterized_range_fires_with_both_bound_parameters() {
    // BRIEF-154 bar 2: `WHERE n.x > $lo AND n.x < $hi` plans as
    // `Range { lo: Parameter, hi: Parameter, .. }`.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age > $lo AND n.age < $hi RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    let TypedIndexBounds::Range {
        lo,
        lo_inclusive,
        hi,
        hi_inclusive,
    } = bounds
    else {
        panic!("expected parameterized range bound, got {bounds:?}");
    };
    let IndexKey::Parameter { name: lo_name, .. } = lo else {
        panic!("expected parameter lo bound, got {lo:?}");
    };
    let IndexKey::Parameter { name: hi_name, .. } = hi else {
        panic!("expected parameter hi bound, got {hi:?}");
    };
    assert_eq!(lo_name.as_str(), "lo");
    assert_eq!(hi_name.as_str(), "hi");
    assert!(!*lo_inclusive);
    assert!(!*hi_inclusive);
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn parameterized_range_mixed_literal_and_parameter_fires() {
    // BRIEF-154 bar 2 (mixed): literal + parameter Range still fires.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age >= $lo AND n.age < 100 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    let TypedIndexBounds::Range { lo, hi, .. } = bounds else {
        panic!("expected mixed range bound, got {bounds:?}");
    };
    assert!(matches!(lo, IndexKey::Parameter { .. }));
    assert!(matches!(
        hi,
        IndexKey::Literal(selene_gql::Literal::Integer(100, _))
    ));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn typed_parameter_with_incompatible_declaration_falls_back_to_linear() {
    // BRIEF-154 bar 6: `$id :: INT` against a STRING-indexed column → Linear,
    // not an error. Untyped parameters wouldn't trigger this fallback; the
    // typed declaration is what the plan-time check uses to reject early.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.name = $id :: INTEGER RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(
        matches!(scan.access, ScanAccess::Linear),
        "expected linear fallback for typed-incompatible parameter, got {:?}",
        scan.access
    );
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn float_width_generic_typed_param_against_float_index_falls_back_to_linear() {
    // BRIEF-154 PR #175 F2 (Codex P2): a `$p :: FLOAT` declaration would
    // accept both `Value::Float` (f64) and `Value::Float32` per
    // `parameter_type::validate_declared_type`, but `check_value_index_kind`
    // only admits `Value::Float` for `IndexKind::Float`. Admitting
    // `GqlType::Float` for `IndexKind::Float` at plan time would let the
    // indexed path optimize through, then error `InvalidParameterType`
    // when the caller binds a `Value::Float32` — while the non-indexed
    // equivalent would compare normally. We avoid that semantic divergence
    // by treating `GqlType::Float` as typed-incompatible with
    // `IndexKind::Float` at plan time, falling back to Linear.
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Person"),
        db_string("score"),
        IndexKind::Float,
    );
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.score = $score :: FLOAT RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(
        matches!(scan.access, ScanAccess::Linear),
        "expected Linear fallback for `$p :: FLOAT` against IndexKind::Float, got {:?}",
        scan.access
    );
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn float64_typed_param_against_float_index_fires() {
    // Companion to the FLOAT fallback: the strict-width `FLOAT64`
    // declaration is unambiguous (only `Value::Float` matches in
    // `parameter_type::validate_declared_type`), so it admits at plan
    // time and the indexed path fires.
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Person"),
        db_string("score"),
        IndexKind::Float,
    );
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.score = $score :: FLOAT64 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(
        matches!(scan.access, ScanAccess::TypedIndexRange { .. }),
        "expected TypedIndexRange for `$p :: FLOAT64`, got {:?}",
        scan.access
    );
}

#[test]
fn float32_typed_param_against_float32_index_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Person"),
        db_string("score"),
        IndexKind::Float32,
    );
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.score = $score :: FLOAT32 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::Float32);
    assert!(matches!(
        bounds,
        TypedIndexBounds::Equality(IndexKey::Parameter {
            declared_type: Some(selene_gql::GqlType::Float32),
            ..
        })
    ));
}

#[test]
fn generic_float_typed_param_against_float32_index_falls_back_to_linear() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Person"),
        db_string("score"),
        IndexKind::Float32,
    );
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.score = $score :: FLOAT RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(
        matches!(scan.access, ScanAccess::Linear),
        "expected Linear fallback for `$p :: FLOAT` against IndexKind::Float32, got {:?}",
        scan.access
    );
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn typed_parameter_with_compatible_declaration_fires() {
    // BRIEF-154 §B.5 happy path: a STRING-typed parameter against a STRING
    // index admits at plan time.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.name = $name :: STRING RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    let TypedIndexBounds::Equality(IndexKey::Parameter {
        name,
        declared_type,
        ..
    }) = bounds
    else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "name");
    assert_eq!(*declared_type, Some(selene_gql::GqlType::String));
}

#[test]
fn boolean_typed_parameter_with_compatible_declaration_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Person"),
        db_string("active"),
        IndexKind::Boolean,
    );
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.active = $active :: BOOLEAN RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::Boolean);
    let TypedIndexBounds::Equality(IndexKey::Parameter {
        name,
        declared_type,
        ..
    }) = bounds
    else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "active");
    assert_eq!(*declared_type, Some(selene_gql::GqlType::Boolean));
}

#[test]
fn uint_typed_parameter_with_compatible_declaration_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Person"),
        db_string("reading_count"),
        IndexKind::UnsignedInteger,
    );
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.reading_count = $count :: UINT64 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::UnsignedInteger);
    let TypedIndexBounds::Equality(IndexKey::Parameter {
        name,
        declared_type,
        ..
    }) = bounds
    else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "count");
    assert_eq!(*declared_type, Some(selene_gql::GqlType::Uint64));
}

#[test]
fn int128_typed_parameter_with_compatible_declaration_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Metric"),
        db_string("signed"),
        IndexKind::Integer128,
    );
    let plan = optimized_one(
        "MATCH (n:Metric) WHERE n.signed = $signed :: INT128 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::Integer128);
    let TypedIndexBounds::Equality(IndexKey::Parameter {
        name,
        declared_type,
        ..
    }) = bounds
    else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "signed");
    assert_eq!(*declared_type, Some(selene_gql::GqlType::Int128));
}

#[test]
fn uint128_typed_parameter_with_compatible_declaration_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Metric"),
        db_string("unsigned"),
        IndexKind::UnsignedInteger128,
    );
    let plan = optimized_one(
        "MATCH (n:Metric) WHERE n.unsigned = $unsigned :: UINT128 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::UnsignedInteger128);
    let TypedIndexBounds::Equality(IndexKey::Parameter {
        name,
        declared_type,
        ..
    }) = bounds
    else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "unsigned");
    assert_eq!(*declared_type, Some(selene_gql::GqlType::Uint128));
}

#[test]
fn decimal_typed_parameter_with_compatible_declaration_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Metric"),
        db_string("amount"),
        IndexKind::Decimal,
    );
    let plan = optimized_one(
        "MATCH (n:Metric) WHERE n.amount = $amount :: DECIMAL RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::Decimal);
    let TypedIndexBounds::Equality(IndexKey::Parameter {
        name,
        declared_type,
        ..
    }) = bounds
    else {
        panic!("expected parameterized equality bound, got {bounds:?}");
    };
    assert_eq!(name.as_str(), "amount");
    assert_eq!(*declared_type, Some(selene_gql::GqlType::Decimal));
}

#[test]
fn decimal_literal_with_decimal_index_fires() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        db_string("Metric"),
        db_string("amount"),
        IndexKind::Decimal,
    );
    let plan = optimized_one("MATCH (n:Metric) WHERE n.amount = 1.25 RETURN n", &catalog);
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected typed-index range, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::Decimal);
    assert!(matches!(
        bounds,
        TypedIndexBounds::Equality(IndexKey::Literal(selene_gql::Literal::Decimal(..)))
    ));
}

#[test]
fn contradictory_combined_bounds_keep_residual_predicate() {
    // `age > 10 AND age < 5` is empty. When `bounds_for_property` would
    // produce a contradictory combined Range (lo=10, hi=5), it refuses the
    // combined rewrite; the rule then falls back to single-predicate index
    // access, leaving the other predicate as a residual filter so the
    // executor can prove the empty result.
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age > 10 AND n.age < 5 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(
        matches!(scan.access, ScanAccess::TypedIndexRange { .. }),
        "expected typed-index range; never the weaker single bound applied to both predicates"
    );
    // Exactly one predicate should remain as residual; the other was consumed.
    assert_eq!(scan.property_predicates.len(), 1);
}
