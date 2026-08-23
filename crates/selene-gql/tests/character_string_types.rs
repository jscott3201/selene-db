//! Character-string type descriptor coverage for `STRING`, `CHAR`, and `VARCHAR`.

use selene_core::{GraphId, Record, Value};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, ImplDefinedCaps, Session,
    StatementOutput, analyze, execute_statement, feature_walk, parse, plan,
};
use selene_graph::{GraphTypeDef, SharedGraph};
use selene_profile::FeatureId;
use smallvec::smallvec;

fn db_string(value: &str) -> selene_core::DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source);
    execute_statement(&plan, session, &EmptyProcedureRegistry)
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(16_200));
    let mut session = Session::new(&graph);
    let table = rows(execute(source, &mut session).expect("statement succeeds"));
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(16_201));
    let mut session = Session::new(&graph);
    execute(source, &mut session)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("character.string.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn assert_feature_recorded(source: &str, expected: FeatureId) {
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&expected),
        "{source} should record {expected:?}, observed {observed:?}"
    );
}

#[test]
fn character_string_forms_parse_flag_and_format() {
    for (source, expected) in [
        ("RETURN 'abc' IS TYPED STRING(4) AS ok", FeatureId::GV31),
        ("RETURN 'abc' IS TYPED STRING(1, 4) AS ok", FeatureId::GV30),
        ("RETURN 'a' IS TYPED CHAR AS ok", FeatureId::GV32),
        ("RETURN 'abc' IS TYPED CHAR(3) AS ok", FeatureId::GV32),
        ("RETURN 'abc' IS TYPED VARCHAR(4) AS ok", FeatureId::GV31),
    ] {
        parse(source).unwrap_or_else(|err| panic!("character-string type parses: {err:?}"));
        assert_feature_recorded(source, expected);
    }

    let fixed = parse("RETURN 'ab' IS TYPED CHAR(2) AS ok").expect("source parses");
    let formatted = selene_gql::ast::format_read_statement(&fixed).expect("formats");
    assert_eq!(formatted, "RETURN 'ab' IS TYPED STRING(2, 2) AS ok");

    let variable = parse("RETURN 'ab' IS TYPED VARCHAR(4) AS ok").expect("source parses");
    let formatted = selene_gql::ast::format_read_statement(&variable).expect("formats");
    assert_eq!(formatted, "RETURN 'ab' IS TYPED STRING(4) AS ok");
}

#[test]
fn character_string_type_predicates_check_character_bounds() {
    assert_eq!(
        first_value("RETURN 'abc' IS TYPED STRING(2, 4) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN 'a' IS TYPED STRING(2, 4) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN 'abcde' IS TYPED STRING(2, 4) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN 'x' IS TYPED CHAR AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN 'xy' IS TYPED CHAR AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn character_string_cast_applies_bounds() {
    assert_eq!(
        first_value("RETURN CAST('a' AS CHAR(3)) AS value"),
        Value::String(db_string("a  "))
    );
    assert_eq!(
        first_value("RETURN CAST('abc  ' AS STRING(3)) AS value"),
        Value::String(db_string("abc"))
    );
    assert_eq!(
        first_status("RETURN CAST('abcd' AS VARCHAR(3)) AS value"),
        "22001"
    );
}

#[test]
fn character_string_catalog_round_trips_through_show_and_validates_writes() {
    let graph = empty_closed_graph(16_202);
    let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);

    execute(
        "CREATE NODE TYPE :Doc (\
            title :: VARCHAR(4) DEFAULT 'note', \
            tags :: LIST<STRING(1, 3)> DEFAULT ['a', 'bc'], \
            meta :: RECORD { code :: CHAR(2) } DEFAULT RECORD{code: 'id'}\
        )",
        &mut session,
    )
    .expect("catalog succeeds");

    let table = rows(execute("SHOW NODE TYPES", &mut session).expect("show succeeds"));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Doc (title :: STRING(4) DEFAULT 'note', tags :: LIST<STRING(1, 3)> DEFAULT ['a', 'bc'], meta :: RECORD { code :: STRING(2, 2) } DEFAULT RECORD{code: 'id'})"
        ))
    );

    execute(
        "INSERT (:Doc {title: 'okay', tags: ['a', 'bc'], meta: RECORD{code: 'id'}}) FINISH",
        &mut session,
    )
    .expect("bounded character strings are assignable");
    execute(
        "INSERT (:Doc {title: 'too long', tags: ['a'], meta: RECORD{code: 'id'}}) FINISH",
        &mut session,
    )
    .expect_err("overlength title violates STRING(4)");
}

#[test]
fn character_string_store_assignment_pads_and_truncates_whitespace() {
    let graph = empty_closed_graph(16_204);
    let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);

    execute(
        "CREATE NODE TYPE :Doc (\
            title :: CHAR(3), \
            tags :: LIST<STRING(2, 3)>, \
            meta :: RECORD { code :: CHAR(2) }\
        )",
        &mut session,
    )
    .expect("catalog succeeds");
    execute(
        "INSERT (:Doc {title: 'a', tags: ['x', 'yz  '], meta: RECORD{code: 'q'}}) FINISH",
        &mut session,
    )
    .expect("store assignment pads and truncates whitespace");

    let table = rows(
        execute(
            "MATCH (n:Doc) RETURN n.title AS title, n.tags AS tags, n.meta AS meta",
            &mut session,
        )
        .expect("match succeeds"),
    );
    assert_eq!(
        table.rows()[0].values(),
        &[
            Value::String(db_string("a  ")),
            Value::List(vec![
                Value::String(db_string("x ")),
                Value::String(db_string("yz ")),
            ]),
            Value::Record(Box::new(Record::Open(smallvec![(
                db_string("code"),
                Value::String(db_string("q ")),
            )]))),
        ]
    );

    execute("MATCH (n:Doc) SET n.title = 'xy' FINISH", &mut session)
        .expect("SET applies store assignment");
    let table = rows(
        execute("MATCH (n:Doc) RETURN n.title AS title", &mut session).expect("match succeeds"),
    );
    assert_eq!(table.rows()[0].values(), &[Value::String(db_string("xy "))]);

    let err = execute("MATCH (n:Doc) SET n.title = 'toolong' FINISH", &mut session)
        .expect_err("non-space truncation errors");
    assert_eq!(err.gqlstatus().as_str(), "22001");
}

#[test]
fn character_string_defaults_are_store_assigned() {
    let graph = empty_closed_graph(16_203);
    let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);

    execute(
        "CREATE NODE TYPE :Doc (\
            title :: CHAR(3) DEFAULT 'a', \
            tags :: LIST<STRING(2, 3)> DEFAULT ['x', 'yz  '], \
            meta :: RECORD { code :: CHAR(2) } DEFAULT RECORD{code: 'q'}\
        )",
        &mut session,
    )
    .expect("DEFAULT descriptor coercion succeeds");

    let table = rows(execute("SHOW NODE TYPES", &mut session).expect("SHOW succeeds"));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Doc (title :: STRING(3, 3) DEFAULT 'a  ', tags :: LIST<STRING(2, 3)> DEFAULT ['x ', 'yz '], meta :: RECORD { code :: STRING(2, 2) } DEFAULT RECORD{code: 'q '})"
        ))
    );

    execute("INSERT (:Doc) FINISH", &mut session).expect("insert materializes defaults");
    let table = rows(
        execute(
            "MATCH (n:Doc) RETURN n.title AS title, n.tags AS tags, n.meta AS meta",
            &mut session,
        )
        .expect("match succeeds"),
    );
    assert_eq!(
        table.rows()[0].values(),
        &[
            Value::String(db_string("a  ")),
            Value::List(vec![
                Value::String(db_string("x ")),
                Value::String(db_string("yz ")),
            ]),
            Value::Record(Box::new(Record::Open(smallvec![(
                db_string("code"),
                Value::String(db_string("q ")),
            )]))),
        ]
    );
}

/// Cross-funnel IV023 pin: a discarded trailing character is `<truncating
/// whitespace>` only when it is U+0020. Character CAST, INSERT/SET store
/// assignment, and DEFAULT descriptor coercion must produce identical
/// accept/reject decisions for the same source value; a divergence here is a
/// per-funnel policy fork (`CAST('ab\t' AS VARCHAR(2))` succeeding while
/// `SET n.p = 'ab\t'` raises 22001), not a tolerable difference.
#[test]
fn truncating_whitespace_policy_is_identical_across_funnels() {
    // GQL escape-text suffixes appended to 'ab' against a length-2 envelope.
    let cases: [(&str, bool); 6] = [
        (" ", true),
        ("  ", true),
        (r"\t", false),
        (r"\n", false),
        (r"\u00A0", false),
        // A mixed tail is rejected by its single data-bearing character even
        // though it also contains discardable spaces.
        (r" \t ", false),
    ];

    for (suffix, accepted) in cases {
        // (a) Character CAST.
        for target in ["VARCHAR(2)", "CHAR(2)"] {
            let source = format!("RETURN CAST('ab{suffix}' AS {target}) AS value");
            if accepted {
                assert_eq!(
                    first_value(&source),
                    Value::String(db_string("ab")),
                    "CAST AS {target} accepts space-only tail {suffix:?}"
                );
            } else {
                assert_eq!(
                    first_status(&source),
                    "22001",
                    "CAST AS {target} rejects data-bearing tail {suffix:?}"
                );
            }
        }

        // (b) INSERT and SET store assignment on a closed graph.
        let graph = empty_closed_graph(16_206);
        let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);
        execute("CREATE NODE TYPE :Doc (p :: VARCHAR(2))", &mut session).expect("catalog succeeds");
        execute("INSERT (:Doc {p: 'ok'}) FINISH", &mut session).expect("seed row inserts");

        let insert = format!("INSERT (:Doc {{p: 'ab{suffix}'}}) FINISH");
        let set = format!("MATCH (n:Doc) SET n.p = 'ab{suffix}' FINISH");
        if accepted {
            execute(&insert, &mut session).unwrap_or_else(|err| {
                panic!("INSERT discards space-only tail {suffix:?}: {err:?}")
            });
            execute(&set, &mut session)
                .unwrap_or_else(|err| panic!("SET discards space-only tail {suffix:?}: {err:?}"));
            let table = rows(
                execute("MATCH (n:Doc) RETURN n.p AS p", &mut session).expect("match succeeds"),
            );
            assert_eq!(table.row_count(), 2);
            for row in table.rows() {
                assert_eq!(
                    row.values(),
                    &[Value::String(db_string("ab"))],
                    "store assignment truncates {suffix:?} to the envelope"
                );
            }
        } else {
            for source in [insert.as_str(), set.as_str()] {
                let err = execute(source, &mut session)
                    .expect_err("store assignment rejects data-bearing tail");
                assert_eq!(err.gqlstatus().as_str(), "22001", "for {source}");
            }
            // Failed writes must not leak partial state: the seed row is
            // unchanged and the rejected INSERT produced no row.
            let table = rows(
                execute("MATCH (n:Doc) RETURN n.p AS p", &mut session).expect("match succeeds"),
            );
            assert_eq!(table.row_count(), 1);
            assert_eq!(table.rows()[0].values(), &[Value::String(db_string("ok"))]);
        }

        // (c) DEFAULT descriptor coercion.
        for target in ["VARCHAR(2)", "CHAR(2)"] {
            let graph = empty_closed_graph(16_207);
            let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);
            let ddl = format!("CREATE NODE TYPE :Spec (p :: {target} DEFAULT 'ab{suffix}')");
            if accepted {
                execute(&ddl, &mut session).unwrap_or_else(|err| {
                    panic!("DEFAULT discards space-only tail {suffix:?}: {err:?}")
                });
                execute("INSERT (:Spec) FINISH", &mut session).expect("default materializes");
                let table = rows(
                    execute("MATCH (n:Spec) RETURN n.p AS p", &mut session)
                        .expect("match succeeds"),
                );
                assert_eq!(
                    table.rows()[0].values(),
                    &[Value::String(db_string("ab"))],
                    "{target} DEFAULT truncates {suffix:?} to the envelope"
                );
            } else {
                let err =
                    execute(&ddl, &mut session).expect_err("DEFAULT rejects data-bearing tail");
                assert_eq!(err.gqlstatus().as_str(), "22001", "for {ddl}");
            }
        }
    }
}

#[test]
fn character_string_defaults_reject_non_space_truncation() {
    for source in [
        "CREATE NODE TYPE :Doc (title :: STRING(2, 4) DEFAULT 'abcde')",
        "CREATE NODE TYPE :Doc (tags :: LIST<STRING(2, 4)> DEFAULT ['abcde'])",
        "CREATE NODE TYPE :Doc (meta :: RECORD { title :: STRING(2, 4) } DEFAULT RECORD{title: 'abcde'})",
    ] {
        let graph = empty_closed_graph(16_205);
        let mut session = Session::new(&graph);
        let err = execute(source, &mut session).expect_err("DEFAULT truncates non-space data");
        assert_eq!(err.gqlstatus().as_str(), "22001");
        assert!(
            err.to_string().contains("DEFAULT"),
            "expected DEFAULT validation error for `{source}`, got {err:?}"
        );
    }
}
