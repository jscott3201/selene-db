use super::*;

#[test]
fn typed_parameter_ast_shapes_cover_value_limit_and_list() {
    let statement = parse("RETURN $id :: INTEGER AS id").expect("typed return parses");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    let ValueExpr::Parameter {
        name,
        declared_type: Some(GqlType::Integer),
        ..
    } = &clause.items[0].expr
    else {
        panic!("expected typed parameter expression");
    };
    assert_eq!(name.as_str(), "id");

    let statement = parse("MATCH (n) RETURN n LIMIT $count :: INT").expect("typed LIMIT parses");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Limit(LimitValue::Parameter {
        name,
        declared_type: Some(GqlType::Integer),
        ..
    }) = &pipeline.statements[2]
    else {
        panic!("expected typed LIMIT parameter");
    };
    assert_eq!(name.as_str(), "count");

    let statement = parse("RETURN $vals :: LIST<INT> AS vals").expect("typed list parses");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    let ValueExpr::Parameter {
        declared_type: Some(GqlType::List(inner)),
        ..
    } = &clause.items[0].expr
    else {
        panic!("expected LIST typed parameter expression");
    };
    assert_eq!(**inner, GqlType::Integer);
}

#[test]
fn untyped_parameter_ast_and_analyzer_stay_dynamic() {
    let statement = parse("RETURN $id AS id").expect("untyped parameter parses");
    let Statement::Query(pipeline) = &statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    assert!(matches!(
        &clause.items[0].expr,
        ValueExpr::Parameter {
            name,
            declared_type: None,
            ..
        } if name.as_str() == "id"
    ));

    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("source analyzes");
    assert_eq!(projection_type(&analyzed, "id"), AnalyzedType::Dynamic);
}

#[test]
fn typed_parameter_declaration_inherits_to_bare_references() {
    let analyzed = analyze_one("RETURN $x :: INT AS \"typed\", $x AS bare");
    assert_eq!(
        projection_type(&analyzed, "typed"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(
        projection_type(&analyzed, "bare"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn typed_parameter_conflicts_are_analyzer_errors() {
    let err = analyze(
        parse("MATCH (n {a: $x :: INT, b: $x :: STRING}) RETURN n").expect("source parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect_err("conflicting declarations reject");
    assert!(matches!(
        err,
        AnalysisError::ConflictingParameterTypes {
            ref name,
            ref declarations,
        } if name.as_str() == "x"
            && declarations.len() == 2
            && declarations[0].0 == GqlType::Integer
            && declarations[1].0 == GqlType::String
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
}

#[test]
fn typed_parameter_conflicts_cross_value_and_limit_surfaces() {
    let err = analyze(
        parse("MATCH (n {a: $x :: INT}) RETURN n LIMIT $x :: STRING").expect("source parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect_err("cross-surface conflict rejects");
    assert!(matches!(
        err,
        AnalysisError::ConflictingParameterTypes { name, .. } if name.as_str() == "x"
    ));
}

#[test]
fn typed_limit_parameter_rejects_unsatisfiable_declared_types_at_analysis() {
    for source in [
        "RETURN 1 AS n LIMIT $rows :: STRING",
        "RETURN 1 AS n OFFSET $rows :: FLOAT",
    ] {
        let err = analyze(
            parse(source).expect("source parses"),
            &EmptyProcedureRegistry,
            None,
        )
        .expect_err("unsatisfiable LIMIT/OFFSET declaration rejects");
        assert!(matches!(
            err,
            AnalysisError::TypeMismatch {
                context: TypeMismatchContext::LimitAmount,
                expected: ExpectedType::LimitAmount,
                ..
            }
        ));
        assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
    }
}

#[test]
fn typed_parameter_runtime_rejects_bare_reference_inherited_from_declaration() {
    let graph = SharedGraph::new(GraphId::new(4120));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("x"), Value::String(db_string("abc")));

    let err = execute(
        &mut session,
        "RETURN CASE WHEN false THEN $x :: INT ELSE $x END AS value",
    )
    .expect_err("bare reference inherits declared type");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "x" && expected == "INTEGER"
    ));
}

#[test]
fn typed_parameter_runtime_accepts_and_rejects_declared_value_type() {
    let graph = SharedGraph::new(GraphId::new(4117));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("id"), Value::Int(42));
    assert_eq!(
        single_value(&mut session, "RETURN $id :: INT AS id"),
        Value::Int(42)
    );

    session.bind_parameter(db_string("id"), Value::String(db_string("abc")));
    let err = execute(&mut session, "RETURN $id :: INT AS id")
        .expect_err("typed parameter rejects mismatched value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "id" && expected == "INTEGER"
    ));
}

#[test]
fn typed_parameter_python_shape_fixture() {
    let graph = SharedGraph::new(GraphId::new(4121));
    let mut session = Session::new(&graph);
    let mut params = HashMap::new();
    params.insert("id", serde_json::json!(42));
    params.insert("name", serde_json::json!("thermostat"));

    bind_json_params(&mut session, params);

    let table = rows(
        execute(
            &mut session,
            "INSERT (n:Sensor {id: $id :: INT, name: $name :: STRING}) \
             RETURN n.id AS id, n.name AS name",
        )
        .expect("typed parameter insert executes"),
    );
    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(42), Value::String(db_string("thermostat"))]
    );
}

#[test]
fn typed_limit_parameter_runtime_checks_declared_type_first() {
    let graph = graph_with_sensors(4118);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count"), Value::Float(3.0));

    let err = execute(
        &mut session,
        "MATCH (n:Sensor) RETURN n LIMIT $count :: INT",
    )
    .expect_err("typed LIMIT parameter rejects mismatched value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "FLOAT64",
            ..
        } if name.as_str() == "count" && expected == "INTEGER"
    ));
}

#[test]
fn typed_parameter_plan_cache_reuses_same_type_and_partitions_different_types() {
    let graph = SharedGraph::new(GraphId::new(4119));
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(8).unwrap());
    session.bind_parameter(db_string("a"), Value::Int(1));
    assert_eq!(
        single_value(&mut session, "RETURN $a :: INT AS a"),
        Value::Int(1)
    );
    session.bind_parameter(db_string("a"), Value::Int(2));
    assert_eq!(
        single_value(&mut session, "RETURN $a :: INT AS a"),
        Value::Int(2)
    );
    session.bind_parameter(db_string("a"), Value::String(db_string("x")));
    assert_eq!(
        single_value(&mut session, "RETURN $a :: STRING AS a"),
        Value::String(db_string("x"))
    );

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 1);
}
