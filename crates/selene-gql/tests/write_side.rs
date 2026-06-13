//! Write-side parser coverage for BRIEF-18.

use selene_gql::{
    DdlStatement, DeleteMode, DropBehavior, GqlStatus, GqlType, MutationStatement,
    MutationTerminator, ParserError, PipelineStatement, Statement, TypePropertyConstraint,
    ValidationMode, YieldColumn, parse,
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
    // CREATE GRAPH stays parse-rejected under D1 single-graph (GC04 unsupported
    // — cannot create a second graph).
    for source in ["CREATE GRAPH foo", "CREATE GRAPH IF NOT EXISTS foo"] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
    }
    // DROP GRAPH now parses (BRIEF-152): it is the IM_DROP_GRAPH factory-reset
    // extension, reaching the executor instead of dying in the flagger. Under
    // D1 the parsed name is informational and IF EXISTS is trivially satisfied.
    for source in ["DROP GRAPH foo", "DROP GRAPH IF EXISTS foo"] {
        let DdlStatement::DropGraph { if_exists, .. } = parse_ddl(source) else {
            panic!("expected DROP GRAPH DDL for {source}");
        };
        assert_eq!(if_exists, source.contains("IF EXISTS"), "{source}");
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
    assert!(matches!(
        &properties[1].gql_type,
        GqlType::NotNull(inner) if **inner == GqlType::Integer
    ));
    assert!(properties[1].constraints.is_empty());

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
        label,
        extends,
        endpoints,
        properties,
        ..
    } = parse_ddl(
        "CREATE EDGE TYPE :KNOWS EXTENDS :RELATIONSHIP (FROM :Person TO :Person, since :: DATE)",
    )
    else {
        panic!("expected CREATE EDGE TYPE");
    };
    assert_eq!(label.as_str(), "KNOWS");
    assert_eq!(extends.expect("parent").as_str(), "RELATIONSHIP");
    let endpoints = endpoints.expect("endpoint spec");
    assert_eq!(endpoints.from_labels[0].as_str(), "Person");
    assert_eq!(properties[0].gql_type, GqlType::Date);

    // Default (no behavior keyword) lowers to RESTRICT.
    assert!(matches!(
        parse_ddl("DROP NODE TYPE IF EXISTS :Person"),
        DdlStatement::DropNodeType {
            if_exists: true,
            behavior: DropBehavior::Restrict,
            ..
        }
    ));
    assert!(matches!(
        parse_ddl("DROP EDGE TYPE :KNOWS"),
        DdlStatement::DropEdgeType {
            behavior: DropBehavior::Restrict,
            ..
        }
    ));
    // Explicit CASCADE / RESTRICT keywords are carried on the AST.
    assert!(matches!(
        parse_ddl("DROP NODE TYPE :Person CASCADE"),
        DdlStatement::DropNodeType {
            behavior: DropBehavior::Cascade,
            ..
        }
    ));
    assert!(matches!(
        parse_ddl("DROP EDGE TYPE :KNOWS RESTRICT"),
        DdlStatement::DropEdgeType {
            behavior: DropBehavior::Restrict,
            ..
        }
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
    // The ISO/IEC 39075:2024 §18 property constraints the engine accepts:
    // DEFAULT, IMMUTABLE, UNIQUE, INDEXED. Explicit value-type nullability is
    // carried on the GqlType and lowered to the required-property bit at the
    // catalog boundary. Donor full-text/time-series constraints (SEARCHABLE/
    // DICTIONARY/FILL/INTERVAL/ENCODING) were removed from the grammar — see
    // `donor_property_constraints_are_syntax_errors`.
    let DdlStatement::CreateNodeType { properties, .. } = parse_ddl(
        "CREATE NODE TYPE :Sensor (v :: STRING NOT NULL DEFAULT 'x' IMMUTABLE UNIQUE INDEXED)",
    ) else {
        panic!("expected CREATE NODE TYPE");
    };
    assert!(matches!(
        &properties[0].gql_type,
        GqlType::NotNull(inner) if **inner == GqlType::String
    ));
    assert_eq!(properties[0].constraints.len(), 4);
}

#[test]
fn donor_property_constraints_are_syntax_errors() {
    // SEARCHABLE/DICTIONARY/FILL/INTERVAL/ENCODING are donor full-text/
    // time-series residue with no place in ISO/IEC 39075:2024; the grammar no
    // longer recognizes them, so they fail as 42001 syntax errors at parse time.
    for source in [
        "CREATE NODE TYPE :S (v :: STRING SEARCHABLE)",
        "CREATE NODE TYPE :S (v :: STRING DICTIONARY)",
        "CREATE NODE TYPE :S (v :: STRING FILL LOCF)",
        "CREATE NODE TYPE :S (v :: STRING INTERVAL '60s')",
        "CREATE NODE TYPE :S (v :: STRING ENCODING RLE)",
    ] {
        let err = selene_gql::parse(source).expect_err("donor constraint should be rejected");
        assert_eq!(
            err.gqlstatus(),
            selene_gql::GqlStatus::SYNTAX_ERROR,
            "expected 42001 for {source:?}"
        );
    }
}

#[test]
fn parse_indexed_constraint_with_explicit_name() {
    let DdlStatement::CreateNodeType { properties, .. } =
        parse_ddl("CREATE NODE TYPE :Sensor (v :: STRING INDEXED AS sensor_v_idx)")
    else {
        panic!("expected CREATE NODE TYPE");
    };
    let TypePropertyConstraint::Indexed {
        name: Some(ref name),
        ..
    } = properties[0].constraints[0]
    else {
        panic!("expected named INDEXED constraint");
    };
    assert_eq!(name.as_str(), "sensor_v_idx");
}

#[test]
fn parse_named_index_ddl() {
    let DdlStatement::CreateIndex {
        name,
        label,
        properties,
        if_not_exists,
        ..
    } = parse_ddl("CREATE INDEX IF NOT EXISTS sensor_ts_idx ON :Sensor(ts, value)")
    else {
        panic!("expected CREATE INDEX");
    };
    assert_eq!(name.as_str(), "sensor_ts_idx");
    assert_eq!(label.as_str(), "Sensor");
    assert_eq!(
        properties
            .iter()
            .map(|property| property.as_str())
            .collect::<Vec<_>>(),
        ["ts", "value"]
    );
    assert!(if_not_exists);

    let DdlStatement::DropIndex {
        name, if_exists, ..
    } = parse_ddl("DROP INDEX IF EXISTS sensor_ts_idx")
    else {
        panic!("expected DROP INDEX");
    };
    assert_eq!(name.as_str(), "sensor_ts_idx");
    assert!(if_exists);
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
    assert!(call.yield_filter.is_none());

    let Statement::Call(call) = parse("CALL pkg.cleanup()").expect("CALL without YIELD parses")
    else {
        panic!("expected top-level CALL");
    };
    assert!(call.yield_items.is_empty());
    assert!(call.yield_filter.is_none());
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
    assert_eq!(
        call.yield_items[1].alias.clone().expect("alias").as_str(),
        "alias"
    );
}

#[test]
fn parse_call_yield_where_filter() {
    let Statement::Call(call) =
        parse("CALL pkg.rank() YIELD score AS s WHERE s >= 0").expect("CALL parses")
    else {
        panic!("expected top-level CALL");
    };

    assert_eq!(call.yield_items.len(), 1);
    assert!(call.yield_filter.is_some());
}

#[test]
fn call_yield_filter_synonym_is_not_iso_syntax() {
    for source in [
        "CALL pkg.rank() YIELD score FILTER score >= 0",
        "CALL pkg.rank() YIELD score FILTER WHERE score >= 0",
    ] {
        let error = parse(source).expect_err(source);
        assert!(
            matches!(error, ParserError::SyntaxError { .. }),
            "{source} should be a syntax error, got {error:?}"
        );
    }
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
    // These remain ISO-legal but are deferred in v1.0, so they parse and the
    // builder rejects them with FEATURE_NOT_SUPPORTED (42N01).
    for source in ["MERGE (n:Person {name: 'X'})", "FOR x IN [1,2,3] RETURN x"] {
        let error = parse(source).expect_err(source);
        assert_eq!(
            error.gqlstatus(),
            GqlStatus::FEATURE_NOT_SUPPORTED,
            "{source}"
        );
    }
}

#[test]
fn removed_non_iso_grammar_is_syntax_error() {
    // Triggers, materialized views, procedure DDL, and auth (users/roles/grants)
    // are out of spec entirely (auth = embedder concern D1; procedures are native
    // built-ins; triggers/views are not in the v1.0 claim list). They have no
    // ISO equivalent and were removed from the grammar, so they now fail to
    // parse with SYNTAX_ERROR (42601) rather than FEATURE_NOT_SUPPORTED.
    for source in [
        "CREATE TRIGGER trig AFTER INSERT ON :Person EXECUTE SET n.x = 1",
        "CREATE MATERIALIZED VIEW v AS MATCH (n) RETURN n",
        "CREATE PROCEDURE pkg.fn() { RETURN 1 }",
        "CREATE USER alice SET PASSWORD 'pw'",
        "CREATE ROLE admin",
        "GRANT ROLE admin TO alice",
        "REVOKE ROLE admin FROM alice",
        "DROP TRIGGER trig",
        "SHOW TRIGGERS",
        "MATCH VIEW v YIELD x",
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn call_where_without_yield_is_syntax_error() {
    let error = parse("CALL f() WHERE x = 1").expect_err("CALL WHERE without YIELD");
    assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR);
}
