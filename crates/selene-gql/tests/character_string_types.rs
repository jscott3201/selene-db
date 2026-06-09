//! Character-string type descriptor coverage for `STRING`, `CHAR`, and `VARCHAR`.

use selene_core::{GraphId, Value, feature_register::FeatureId};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, ImplDefinedCaps, Session,
    StatementOutput, analyze, execute_statement, feature_walk, parse, plan,
};
use selene_graph::{GraphTypeDef, SharedGraph};

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
fn character_string_defaults_validate_top_level_list_and_record_bounds() {
    for source in [
        "CREATE NODE TYPE :Doc (title :: STRING(2, 4) DEFAULT 'a')",
        "CREATE NODE TYPE :Doc (tags :: LIST<STRING(2, 4)> DEFAULT ['a'])",
        "CREATE NODE TYPE :Doc (meta :: RECORD { title :: STRING(2, 4) } DEFAULT RECORD{title: 'a'})",
    ] {
        let graph = empty_closed_graph(16_203);
        let mut session = Session::new(&graph);
        let err = execute(source, &mut session).expect_err("DEFAULT violates string bounds");
        assert!(
            err.to_string().contains("DEFAULT"),
            "expected DEFAULT validation error for `{source}`, got {err:?}"
        );
    }
}
