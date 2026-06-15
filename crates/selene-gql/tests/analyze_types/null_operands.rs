use super::*;

#[test]
fn analyze_accepts_null_comparison_operand() {
    let analyzed = analyze_one("RETURN NULL < 5 AS r").expect("NULL < 5 analyzes");
    // Comparison with a known operand still resolves to Boolean (the NULL truth
    // value is a Boolean-domain result, surfaced at runtime as NULL).
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    // Symmetric: NULL on the right of a literal.
    analyze_one("RETURN 5 < NULL AS r").expect("5 < NULL analyzes");
    // NULL on both sides.
    analyze_one("RETURN NULL < NULL AS r").expect("NULL < NULL analyzes");
}

#[test]
fn analyze_accepts_null_arithmetic_operand() {
    // One concrete operand makes the static result Dynamic (the NULL operand has
    // no numeric kind to promote against); the point is that it does NOT error.
    let analyzed = analyze_one("RETURN NULL + 1 AS r").expect("NULL + 1 analyzes");
    assert_eq!(projection_type(&analyzed, "r"), AnalyzedType::Dynamic);
    analyze_one("RETURN 1 + NULL AS r").expect("1 + NULL analyzes");
    analyze_one("RETURN NULL * 2 AS r").expect("NULL * 2 analyzes");
}

#[test]
fn analyze_accepts_unary_negate_over_null() {
    let analyzed = analyze_one("RETURN -NULL AS r").expect("-NULL analyzes");
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Null)
    );
}

#[test]
fn analyze_accepts_null_boolean_operand() {
    let analyzed = analyze_one("RETURN NULL AND true AS r").expect("NULL AND true analyzes");
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    analyze_one("RETURN true OR NULL AS r").expect("true OR NULL analyzes");
    analyze_one("RETURN NULL XOR NULL AS r").expect("NULL XOR NULL analyzes");
}

#[test]
fn analyze_accepts_unary_not_over_null() {
    let analyzed = analyze_one("RETURN NOT NULL AS r").expect("NOT NULL analyzes");
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}
