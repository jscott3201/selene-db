//! Analyzer ProcedureRegistry integration tests.

use selene_core::{IStr, intern};
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedType, BindingDeclKind, EmptyProcedureRegistry,
    GqlStatus, GqlType, ProcedureOutputColumn, ProcedureParameter, ProcedureRegistry,
    TypeMismatchContext, analyze, parse,
};
use selene_testing::MockProcedureRegistry;

fn analyze_with(
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry)
}

fn registry(
    name: &[&str],
    parameters: Vec<ProcedureParameter>,
    output_columns: Vec<ProcedureOutputColumn>,
) -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure(
        name.iter().map(|segment| istr(segment)).collect(),
        parameters,
        output_columns,
    )
}

fn param(name: &str, ty: GqlType, nullable: bool) -> ProcedureParameter {
    ProcedureParameter {
        name: istr(name),
        ty,
        nullable,
    }
}

fn output(name: &str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn {
        name: istr(name),
        ty,
    }
}

fn yield_type(analyzed: &AnalyzedStatement, name: &str) -> AnalyzedType {
    analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.kind() == BindingDeclKind::YieldColumn && decl.name().as_str() == name)
        .unwrap_or_else(|| panic!("yield column {name} exists"))
        .ty()
        .clone()
}

fn yield_names(analyzed: &AnalyzedStatement) -> Vec<&'static str> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .filter(|decl| decl.kind() == BindingDeclKind::YieldColumn)
        .map(|decl| decl.name().as_str())
        .collect()
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test strings fit interner")
}

#[test]
fn call_with_registered_procedure_resolves() {
    let registry = registry(
        &["pkg", "typed"],
        vec![
            param("amount", GqlType::Integer, false),
            param("name", GqlType::String, false),
        ],
        vec![
            output("outA", GqlType::Integer),
            output("outB", GqlType::String),
        ],
    );

    let analyzed = analyze_with("CALL pkg.typed(1, 'hello') YIELD outA, outB", &registry)
        .expect("registered procedure analyzes");

    assert_eq!(
        yield_type(&analyzed, "outA"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(
        yield_type(&analyzed, "outB"),
        AnalyzedType::Resolved(GqlType::String)
    );
}

#[test]
fn yield_star_expands_in_declared_order() {
    let registry = registry(
        &["pkg", "all"],
        Vec::new(),
        vec![
            output("outA", GqlType::Integer),
            output("outB", GqlType::String),
        ],
    );

    let analyzed = analyze_with("CALL pkg.all() YIELD *", &registry).expect("analyzes");

    assert_eq!(yield_names(&analyzed), ["outA", "outB"]);
    assert_eq!(
        yield_type(&analyzed, "outA"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(
        yield_type(&analyzed, "outB"),
        AnalyzedType::Resolved(GqlType::String)
    );
}

#[test]
fn yield_star_plus_explicit_alias_keeps_alias_typed() {
    let registry = registry(
        &["pkg", "all"],
        Vec::new(),
        vec![
            output("outA", GqlType::Integer),
            output("outB", GqlType::String),
        ],
    );

    let analyzed =
        analyze_with("CALL pkg.all() YIELD *, outA AS first", &registry).expect("analyzes");

    assert_eq!(
        yield_type(&analyzed, "first"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(
        yield_type(&analyzed, "outB"),
        AnalyzedType::Resolved(GqlType::String)
    );
}

#[test]
fn yield_star_after_explicit_keeps_wildcard_first_order() {
    let registry = registry(
        &["pkg", "all"],
        Vec::new(),
        vec![
            output("outA", GqlType::Integer),
            output("outB", GqlType::String),
        ],
    );

    let analyzed =
        analyze_with("CALL pkg.all() YIELD outA AS alias, *", &registry).expect("analyzes");

    assert_eq!(yield_names(&analyzed), ["outA", "outB", "alias"]);
    assert_eq!(
        yield_type(&analyzed, "alias"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn argument_type_promotes_via_assignment_direction() {
    let registry = registry(
        &["pkg", "wide"],
        vec![param("value", GqlType::Float64, false)],
        Vec::new(),
    );

    analyze_with("CALL pkg.wide(1)", &registry).expect("integer flows into FLOAT64");
}

#[test]
fn argument_dynamic_defers() {
    let registry = registry(
        &["pkg", "flag"],
        vec![param("flag", GqlType::Boolean, false)],
        vec![output("ok", GqlType::Boolean)],
    );

    let analyzed = analyze_with("CALL pkg.flag($flag) YIELD ok", &registry).expect("analyzes");

    assert_eq!(
        yield_type(&analyzed, "ok"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn top_level_call_statement_analyzes() {
    let registry = registry(&["pkg", "noop"], Vec::new(), Vec::new());
    analyze_with("CALL pkg.noop()", &registry).expect("top-level CALL analyzes");
}

#[test]
fn unknown_procedure_errors() {
    let err = analyze_with("CALL ns.proc()", &EmptyProcedureRegistry).expect_err("unknown");

    assert!(matches!(err, AnalysisError::UnknownProcedure { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_REFERENCE);
}

#[test]
fn wrong_arity_too_few_errors() {
    let registry = registry(
        &["pkg", "need_two"],
        vec![
            param("left", GqlType::Integer, false),
            param("right", GqlType::Integer, false),
        ],
        Vec::new(),
    );

    let err = analyze_with("CALL pkg.need_two(1)", &registry).expect_err("wrong arity");

    assert!(matches!(
        err,
        AnalysisError::WrongArgumentCount {
            expected: 2,
            actual: 1,
            ..
        }
    ));
}

#[test]
fn wrong_arity_too_many_errors() {
    let registry = registry(
        &["pkg", "need_two"],
        vec![
            param("left", GqlType::Integer, false),
            param("right", GqlType::Integer, false),
        ],
        Vec::new(),
    );

    let err = analyze_with("CALL pkg.need_two(1, 2, 3)", &registry).expect_err("wrong arity");

    assert!(matches!(
        err,
        AnalysisError::WrongArgumentCount {
            expected: 2,
            actual: 3,
            ..
        }
    ));
}

#[test]
fn wrong_arity_precedes_argument_binding_errors() {
    let registry = registry(
        &["pkg", "need_one"],
        vec![param("value", GqlType::Integer, false)],
        Vec::new(),
    );

    let err =
        analyze_with("CALL pkg.need_one(undef_var, undef_var2)", &registry).expect_err("arity");

    assert!(matches!(
        err,
        AnalysisError::WrongArgumentCount {
            expected: 1,
            actual: 2,
            ..
        }
    ));
}

#[test]
fn argument_type_mismatch_errors() {
    let registry = registry(
        &["pkg", "booly"],
        vec![param("flag", GqlType::Boolean, false)],
        Vec::new(),
    );

    let err = analyze_with("CALL pkg.booly('no')", &registry).expect_err("type mismatch");

    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::ProcedureArgument { position: 0, .. },
            ..
        }
    ));
}

#[test]
fn null_at_non_nullable_argument_errors() {
    let registry = registry(
        &["pkg", "non_null"],
        vec![param("value", GqlType::Integer, false)],
        Vec::new(),
    );

    let err = analyze_with("CALL pkg.non_null(NULL)", &registry).expect_err("null rejected");

    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::ProcedureArgument { .. },
            ..
        }
    ));
}

#[test]
fn unknown_yield_column_errors() {
    let registry = registry(
        &["pkg", "one"],
        Vec::new(),
        vec![output("known", GqlType::String)],
    );

    let err = analyze_with("CALL pkg.one() YIELD nope", &registry).expect_err("unknown yield");

    assert!(matches!(
        err,
        AnalysisError::UnknownYieldColumn { column, .. } if column.as_str() == "nope"
    ));
}

#[test]
fn empty_registry_rejects_any_call() {
    let err = analyze_with("CALL anything()", &EmptyProcedureRegistry).expect_err("unknown");

    assert!(matches!(err, AnalysisError::UnknownProcedure { .. }));
}

#[test]
fn yield_star_plus_bare_explicit_duplicate_errors() {
    let registry = registry(
        &["pkg", "all"],
        Vec::new(),
        vec![
            output("outA", GqlType::Integer),
            output("outB", GqlType::String),
        ],
    );

    let err = analyze_with("CALL pkg.all() YIELD *, outA", &registry).expect_err("duplicate");

    assert!(matches!(err, AnalysisError::Shadow { name, .. } if name.as_str() == "outA"));
}
