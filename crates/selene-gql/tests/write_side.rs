//! Write-side parser coverage for BRIEF-18.

use selene_gql::{
    DdlStatement, DeleteMode, GqlStatus, GqlType, MutationStatement, MutationTerminator,
    PipelineStatement, Statement, TypePropertyConstraint, ValidationMode, YieldColumn, parse,
};

fn parse_mutation(source: &str) -> selene_gql::MutationPipeline {
    let Statement::Mutate(pipeline) = parse(source).expect("mutation parses") else {
        panic!("expected mutation pipeline");
    };
    pipeline
}

fn parse_ddl(source: &str) -> DdlStatement {
    let Statement::Ddl(statement) = parse(source).expect("DDL parses") else {
        panic!("expected DDL statement");
    };
    statement
}

#[test]
fn parse_insert_single_node() {
    let pipeline = parse_mutation("INSERT (n:Person {name: 'Alice'})");
    let [MutationStatement::Insert(insert)] = pipeline.statements.as_slice() else {
        panic!("expected single INSERT");
    };
    assert_eq!(insert.patterns.len(), 1);
    assert_eq!(insert.patterns[0].elements.len(), 1);
}

#[test]
fn parse_insert_path_and_multiple_patterns() {
    let pipeline = parse_mutation("INSERT (a:Person), (b:Company), (a)-[:WORKS_AT]->(b)");
    let [MutationStatement::Insert(insert)] = pipeline.statements.as_slice() else {
        panic!("expected single INSERT");
    };
    assert_eq!(insert.patterns.len(), 3);
    assert_eq!(insert.patterns[2].elements.len(), 3);
}

#[test]
fn parse_set_variants() {
    let pipeline = parse_mutation(
        "MATCH (n:Person) SET n.age = 30, n.active = true, n = {score: 5}, n :Engineer",
    );
    let [MutationStatement::Match(_), MutationStatement::Set(items)] =
        pipeline.statements.as_slice()
    else {
        panic!("expected MATCH then SET");
    };
    assert_eq!(items.len(), 4);
}

#[test]
fn parse_remove_variants() {
    let pipeline = parse_mutation("MATCH (n) REMOVE n.age, n :Engineer");
    let [
        MutationStatement::Match(_),
        MutationStatement::Remove(items),
    ] = pipeline.statements.as_slice()
    else {
        panic!("expected MATCH then REMOVE");
    };
    assert_eq!(items.len(), 2);
}

#[test]
fn parse_delete_modes() {
    for (source, mode) in [
        ("MATCH (n) DELETE n", DeleteMode::Bare),
        ("MATCH (n) DETACH DELETE n", DeleteMode::Detach),
        ("MATCH (n) NODETACH DELETE n", DeleteMode::NoDetach),
    ] {
        let pipeline = parse_mutation(source);
        let [
            MutationStatement::Match(_),
            MutationStatement::Delete(delete),
        ] = pipeline.statements.as_slice()
        else {
            panic!("expected MATCH then DELETE");
        };
        assert_eq!(delete.mode, mode);
        assert_eq!(delete.items.len(), 1);
    }
}

#[test]
fn parse_finish_terminator() {
    let pipeline = parse_mutation("INSERT (n:Person {name: 'Y'}) FINISH");
    assert!(matches!(
        pipeline.terminator,
        Some(MutationTerminator::Finish(_))
    ));
}

#[test]
fn parse_graph_ddl() {
    for source in [
        "CREATE GRAPH foo",
        "CREATE GRAPH IF NOT EXISTS foo",
        "DROP GRAPH IF EXISTS foo",
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
    }
}

#[test]
fn parse_type_ddl() {
    let DdlStatement::CreateNodeType {
        label, properties, ..
    } = parse_ddl("CREATE NODE TYPE :Person (name :: STRING, age :: INT NOT NULL)")
    else {
        panic!("expected CREATE NODE TYPE");
    };
    assert_eq!(label.as_str(), "Person");
    assert_eq!(properties.len(), 2);
    assert_eq!(properties[1].gql_type, GqlType::Integer);
    assert!(matches!(
        properties[1].constraints.as_slice(),
        [TypePropertyConstraint::NotNull(_)]
    ));

    let DdlStatement::CreateNodeType {
        extends,
        validation_mode,
        ..
    } = parse_ddl("CREATE NODE TYPE IF NOT EXISTS :Person EXTENDS :Entity (id :: STRING) STRICT")
    else {
        panic!("expected CREATE NODE TYPE");
    };
    assert_eq!(extends.expect("parent").as_str(), "Entity");
    assert_eq!(validation_mode, Some(ValidationMode::Strict));
}

#[test]
fn parse_edge_type_and_show_ddl() {
    let DdlStatement::CreateEdgeType {
        endpoints,
        properties,
        ..
    } = parse_ddl("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person, since :: DATE)")
    else {
        panic!("expected CREATE EDGE TYPE");
    };
    let endpoints = endpoints.expect("endpoint spec");
    assert_eq!(endpoints.from_labels[0].as_str(), "Person");
    assert_eq!(properties[0].gql_type, GqlType::Date);

    assert!(matches!(
        parse_ddl("DROP NODE TYPE IF EXISTS :Person"),
        DdlStatement::DropNodeType {
            if_exists: true,
            ..
        }
    ));
    assert!(matches!(
        parse_ddl("DROP EDGE TYPE :KNOWS"),
        DdlStatement::DropEdgeType { .. }
    ));
    assert!(matches!(
        parse_ddl("SHOW NODE TYPES"),
        DdlStatement::ShowNodeTypes(_)
    ));
    assert!(matches!(
        parse_ddl("SHOW EDGE TYPES"),
        DdlStatement::ShowEdgeTypes(_)
    ));
}

#[test]
fn parse_type_property_constraints_exhaustively() {
    let DdlStatement::CreateNodeType { properties, .. } = parse_ddl(
        "CREATE NODE TYPE :Sensor (v :: STRING NOT NULL DEFAULT 'x' IMMUTABLE UNIQUE INDEXED SEARCHABLE DICTIONARY FILL LOCF INTERVAL '60s' ENCODING RLE)",
    ) else {
        panic!("expected CREATE NODE TYPE");
    };
    assert_eq!(properties[0].constraints.len(), 10);
}

#[test]
fn parse_indexed_constraint_with_explicit_name() {
    let DdlStatement::CreateNodeType { properties, .. } =
        parse_ddl("CREATE NODE TYPE :Sensor (v :: STRING INDEXED AS sensor_v_idx)")
    else {
        panic!("expected CREATE NODE TYPE");
    };
    let TypePropertyConstraint::Indexed {
        name: Some(name), ..
    } = properties[0].constraints[0]
    else {
        panic!("expected named INDEXED constraint");
    };
    assert_eq!(name.as_str(), "sensor_v_idx");
}

#[test]
fn parse_top_level_call_variants() {
    let Statement::Call(call) = parse("CALL pkg.fn(1, 'hello') YIELD col1").expect("CALL parses")
    else {
        panic!("expected top-level CALL");
    };
    assert_eq!(
        call.name
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>(),
        ["pkg", "fn"]
    );
    assert_eq!(call.args.len(), 2);
    assert_eq!(call.yield_items.len(), 1);

    let Statement::Call(call) = parse("CALL pkg.cleanup()").expect("CALL without YIELD parses")
    else {
        panic!("expected top-level CALL");
    };
    assert!(call.yield_items.is_empty());
}

#[test]
fn parse_call_yield_star_alias_and_quoted_segment() {
    let Statement::Call(call) =
        parse("CALL foo.\"bar.baz\"() YIELD *, result AS alias").expect("CALL parses")
    else {
        panic!("expected top-level CALL");
    };
    assert_eq!(call.name[1].as_str(), "bar.baz");
    assert!(matches!(call.yield_items[0].column, YieldColumn::Star));
    assert_eq!(call.yield_items[1].alias.expect("alias").as_str(), "alias");
}

#[test]
fn parse_in_pipeline_call() {
    let Statement::Query(query) =
        parse("MATCH (n) CALL pkg.fn(n) YIELD col RETURN col").expect("query with CALL parses")
    else {
        panic!("expected query");
    };
    assert!(matches!(query.statements[1], PipelineStatement::Call(_)));
}

#[test]
fn parse_transaction_control() {
    assert!(matches!(
        parse("START TRANSACTION").expect("parses"),
        Statement::StartTransaction { .. }
    ));
    assert!(matches!(
        parse("COMMIT").expect("parses"),
        Statement::Commit { .. }
    ));
    assert!(matches!(
        parse("ROLLBACK").expect("parses"),
        Statement::Rollback { .. }
    ));
}

#[test]
fn deferred_surfaces_return_not_implemented() {
    for source in [
        "MERGE (n:Person {name: 'X'})",
        "CREATE TRIGGER trig AFTER INSERT ON :Person EXECUTE SET n.x = 1",
        "CREATE MATERIALIZED VIEW v AS MATCH (n) RETURN n",
        "CREATE PROCEDURE pkg.fn() { RETURN 1 }",
        "CREATE INDEX idx ON :Person (name)",
        "CREATE USER alice SET PASSWORD 'pw'",
        "CREATE ROLE admin",
        "GRANT ROLE admin TO alice",
        "CALL { RETURN 1 }",
        "FOR x IN [1,2,3] RETURN x",
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(
            error.gqlstatus(),
            GqlStatus::FEATURE_NOT_SUPPORTED,
            "{source}"
        );
    }
}

#[test]
fn call_where_without_yield_is_syntax_error() {
    let error = parse("CALL f() WHERE x = 1").expect_err("CALL WHERE without YIELD");
    assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR);
}
