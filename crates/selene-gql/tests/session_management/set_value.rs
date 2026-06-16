use super::*;

#[test]
fn set_value_binds_parameter_visible_to_return() {
    let graph = graph(7001);
    let mut session = Session::new(&graph);

    assert!(matches!(
        run(&mut session, "SESSION SET VALUE $p = 42").expect("set value"),
        StatementOutput::Empty
    ));

    let value = single_value(run(&mut session, "RETURN $p").expect("return param"));
    assert_eq!(value, Value::Int(42));
}

#[test]
fn set_value_rebinds_existing_parameter() {
    let graph = graph(7002);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("first bind");

    run(&mut session, "SESSION SET VALUE $p = 2").expect("rebind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $p").expect("return")),
        Value::Int(2)
    );
}

#[test]
fn set_value_rhs_can_reference_prior_parameter() {
    let graph = graph(7003);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $base = 10").expect("base bind");

    // The RHS is restricted to a <value specification>: a bare parameter
    // reference is allowed; a simple expression (GS14) is not.
    run(&mut session, "SESSION SET VALUE $alias = $base").expect("alias bind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $alias").expect("return")),
        Value::Int(10)
    );
}

#[test]
fn set_value_typed_target_parse_carries_declared_type() {
    let statement = parse("SESSION SET VALUE $p :: INTEGER = 42").expect("parse");

    let Statement::SessionSetValue {
        param,
        declared_type,
        ..
    } = statement
    else {
        panic!("expected SESSION SET VALUE");
    };
    assert_eq!(param.as_str(), "p");
    assert_eq!(declared_type, Some(GqlType::Integer));
}

#[test]
fn set_value_accepts_typed_target_spellings() {
    for (index, source) in [
        "SESSION SET VALUE $p INTEGER = 42",
        "SESSION SET VALUE $p TYPED INTEGER = 42",
        "SESSION SET VALUE $p :: INTEGER = 42",
    ]
    .into_iter()
    .enumerate()
    {
        let graph = graph(7030 + index as u64);
        let mut session = Session::new(&graph);

        run(&mut session, source).expect("typed bind");

        assert_eq!(
            single_value(run(&mut session, "RETURN $p").expect("return")),
            Value::Int(42)
        );
    }
}

#[test]
fn set_value_typed_target_rejects_mismatch_without_binding() {
    let graph = graph(7033);
    let mut session = Session::new(&graph);

    let err =
        run(&mut session, "SESSION SET VALUE $p :: INTEGER = 'abc'").expect_err("type mismatch");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "p" && expected.as_ref() == "INTEGER"
    ));
    assert!(matches!(
        run(&mut session, "RETURN $p"),
        Err(ExecutorError::UnboundParameter { .. })
    ));
}

#[test]
fn set_value_if_not_exists_skips_typed_initializer_for_existing_binding() {
    let graph = graph(7034);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("first bind");

    run(
        &mut session,
        "SESSION SET VALUE IF NOT EXISTS $p :: INTEGER = 'abc'",
    )
    .expect("guarded bind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $p").expect("return")),
        Value::Int(1)
    );
}

#[test]
fn set_value_rejects_simple_expression_rhs_gs14() {
    let graph = graph(7006);
    let mut session = Session::new(&graph);

    // Without GS14 the initializer must be a <value specification>; a binary
    // expression is rejected at parse time.
    let err = run(&mut session, "SESSION SET VALUE $p = 1 + 1").expect_err("simple expr rejected");
    assert!(matches!(err, ExecutorError::Parse { .. }));
}

#[test]
fn set_value_if_not_exists_keeps_existing_binding() {
    let graph = graph(7004);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("first bind");

    run(&mut session, "SESSION SET VALUE IF NOT EXISTS $p = 999").expect("guarded bind");

    // The guard must leave the original value untouched.
    assert_eq!(
        single_value(run(&mut session, "RETURN $p").expect("return")),
        Value::Int(1)
    );
}

#[test]
fn set_value_if_not_exists_binds_when_absent() {
    let graph = graph(7005);
    let mut session = Session::new(&graph);

    run(&mut session, "SESSION SET VALUE IF NOT EXISTS $fresh = 7").expect("fresh bind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $fresh").expect("return")),
        Value::Int(7)
    );
}

// ---------------------------------------------------------------------------
// SESSION SET TIME ZONE (GS15) threaded into current-datetime functions
// ---------------------------------------------------------------------------
