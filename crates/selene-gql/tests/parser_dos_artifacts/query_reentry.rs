//! VALUE-query retry pressure and its shared IN/bare-query frame budget.

use std::time::Instant;

use selene_gql::{GqlStatus, ParserError, parse};

use super::PARSE_BUDGET;

// Exact 638-byte timeout-674af2 input from the F06-QUAL-04 CI fuzz log.
// Comments hide most apparent syntax: eleven significant RETURN[VALUE { layers
// remain. Keep the artifact hermetic rather than relying on ignored fuzz files.
const VALUE_QUERY_TIMEOUT: &str = concat!(
    "// corpus//pus:k\n",
    "RETURN[VALUE {//pus: poV08, GV0T32, regular :: UINT, bositiv GV01\n",
    "// also-covers:ok\n",
    "RETURN[VALUE {//puV01\n",
    "// also-covers:ok\n",
    "RETURN[VALUE {//pu08, GV09, GV05, GVTURN[VALUE {//pus:k\n",
    "RETURN[VALUE {//pus: poovers:ok\n",
    "RETURN[VALUE {//pu08, GV09, GV05, GVTURN[VALUE {//pus:k\n",
    "RETURN[VALUE {//pus: poV08, GV09, GV05, GVTURN[VALUE {//pus: po\n",
    "RETURN[VALUE {//pus: poV0T32, regular :: UINT, bigk\n",
    "RETURN[VALUE {//pus: poV08, GV09, GV05, GVTURN[VALUE {//pus: po\n",
    "RETURN[VALUE {//pus: poV0T32, regular :: UINT, big :: po\n",
    "RETURN[VALUE {//pus: poV0T32, regular :: UINT, big ::, , GV03, GV02, GV05, GV06, GV07, G//ok\n",
    "RETURN[VALUE {64)I/N\nD ",
);

pub(super) fn assert_rejected_pre_pest(source: &str) {
    let start = Instant::now();
    let error = parse(source).expect_err("over-budget query re-entry must reject");
    assert!(start.elapsed() < PARSE_BUDGET);
    assert!(
        matches!(error, ParserError::ComplexityLimitExceeded { limit: 8, .. }),
        "expected the shared retry budget, got {error:?}"
    );
    assert_eq!(error.gqlstatus(), GqlStatus::PROGRAM_LIMIT_EXCEEDED);
    // The historical helper name predates grammar factoring. Both malformed
    // and successfully recognized over-budget inputs now use the same public
    // diagnostic, also through the source-backed entry point.
    let report = selene_gql::parse_with_source(std::sync::Arc::from(source), "query.gql")
        .expect_err("source-backed parsing retains the retry limit");
    assert!(matches!(
        report.error(),
        ParserError::ComplexityLimitExceeded { limit: 8, .. }
    ));
    assert_eq!(report.error().gqlstatus().as_str(), "5GQL1");
    assert_eq!(report.source(), source);
}

fn wrap_value_queries(count: usize, body: &str) -> String {
    format!(
        "{}{body}{}",
        "RETURN VALUE {".repeat(count),
        "}".repeat(count)
    )
}

#[test]
fn value_query_timeout_rejects_before_pest() {
    assert_rejected_pre_pest(VALUE_QUERY_TIMEOUT);
    // This minimized regression returned a syntax error after ~2.13 seconds
    // before the guard recognized VALUE-query frames (local debug profile).
    assert_rejected_pre_pest(&format!("{}0", "RETURN [VALUE {".repeat(9)));
}

#[test]
fn value_query_budget_counts_open_frames_through_expression_wrappers() {
    for (prefix, suffix) in [
        ("RETURN [VALUE {", "}]"),
        ("RETURN VALUE {", "}"),
        ("RETURN (VALUE {", "})"),
        ("RETURN {a: VALUE {", "}}"),
        ("RETURN abs(VALUE {", "})"),
        ("ReTuRn [vAlUe /* } ] */ // VALUE {\n {", "}]"),
    ] {
        assert_rejected_pre_pest(&format!("{}0", prefix.repeat(9)));
        assert_rejected_pre_pest(&format!("{}RETURN 0{}", prefix.repeat(9), suffix.repeat(9)));
        parse(&format!("{}RETURN 0{}", prefix.repeat(8), suffix.repeat(8)))
            .expect("exactly eight closed VALUE-query frames remain admitted");
    }
}

#[test]
fn value_in_list_and_bare_query_retries_share_one_budget() {
    assert_rejected_pre_pest(&format!(
        "{}RETURN {}0",
        "RETURN VALUE {".repeat(4),
        "0 IN [".repeat(5)
    ));
    let four_in_lists = format!("RETURN {}0{}", "0 IN [".repeat(4), "]".repeat(4));
    parse(&wrap_value_queries(4, &four_in_lists))
        .expect("four VALUE plus four IN-list frames fit the shared budget");

    // Five bare braces contribute four brace-to-brace retry transitions.
    assert_rejected_pre_pest(&format!("{}{}0", "{".repeat(5), "RETURN VALUE {".repeat(5)));
    parse(&format!(
        "{}{}{}",
        "{".repeat(5),
        wrap_value_queries(4, "RETURN 0"),
        "}".repeat(5)
    ))
    .expect("four bare transitions plus four VALUE frames fit the shared budget");
}

#[test]
fn closed_siblings_release_only_their_own_retry_frames() {
    let siblings = ["VALUE { RETURN [1], {a: 2} }", "0 IN [[0], {a: 1}]"]
        .repeat(10)
        .join(", ");
    parse(&format!(
        "RETURN {}[{siblings}]{}",
        "0 IN [".repeat(7),
        "]".repeat(7)
    ))
    .expect("closed siblings release their frames without accumulating");

    // Ordinary nested maps/lists must not release an enclosing counted frame.
    let body = format!("RETURN [1], {{a: 2}}, {}RETURN 0", "VALUE {".repeat(2));
    assert_rejected_pre_pest(&format!("{}{}", "RETURN VALUE {".repeat(7), body));
}

#[test]
fn value_identifiers_and_quoted_text_do_not_open_query_frames() {
    for body in [
        "RETURN n.VALUE, $VALUE, n.IN, $IN, {VALUE: 1}, 1 AS VALUE",
        "CALL p() YIELD VALUE RETURN VALUE",
        "MATCH (VALUE {x: 1}) RETURN VALUE",
        "MATCH ()-[VALUE {x: 1}]->() RETURN VALUE",
        "MATCH (n:VALUE /* map */ {RETURN /* key */ : 1}) RETURN n",
        "MATCH (VALUE {\"x\": 1}) RETURN VALUE",
        "MATCH (VALUE {`x`: 1}) RETURN VALUE",
        "RETURN 'VALUE { VALUE {', \"VALUE {\", `VALUE {` /* VALUE { */",
    ] {
        parse(&format!(
            "RETURN {}VALUE {{{body}}}{}",
            "0 IN [".repeat(7),
            "]".repeat(7)
        ))
        .unwrap_or_else(|error| panic!("identifier/text at the eight-frame boundary: {error:?}"));
    }
}

#[test]
fn value_edge_quantifiers_keep_their_existing_parse_result() {
    for quantifier in ["1", "1,2", "1,", ",2", "1 /* } */ , // }\n 2"] {
        let body = format!("MATCH ()-[VALUE {{{quantifier}}}]->() RETURN VALUE");
        let expected_status = parse(&body).err().map(|error| error.gqlstatus());
        let source = format!(
            "RETURN {}VALUE {{{body}}}{}",
            "0 IN [".repeat(7),
            "]".repeat(7)
        );
        let actual_status = parse(&source).err().map(|error| error.gqlstatus());
        assert_eq!(actual_status, expected_status);
        assert_ne!(actual_status, Some(GqlStatus::PROGRAM_LIMIT_EXCEEDED));
    }
}

#[test]
fn value_record_type_fields_do_not_open_query_frames() {
    for expr in [
        "CAST(NULL AS {VALUE {x INT}})",
        "CAST(NULL AS {VALUE {RETURN LIST}})",
        "NULL IS TYPED {VALUE {RETURN LIST}}",
        "CAST(NULL AS {other INT, VALUE {RETURN LIST}, CALL {VALUE {x INT}}, USE {VALUE INT}})",
        "CAST(NULL AS LIST<{VALUE {RETURN LIST}}>)",
        "CAST(NULL AS {VALUE TYPED RECORD {RETURN LIST}})",
    ] {
        let source = format!(
            "RETURN {}VALUE {{RETURN {expr}}}{}",
            "0 IN [".repeat(7),
            "]".repeat(7)
        );
        parse(&source).expect("record type fields must not add query retry frames");
    }
    let nested_type = format!("{}{{RETURN LIST}}{}", "{VALUE ".repeat(9), "}".repeat(9));
    for source in [
        format!("RETURN CAST(NULL AS {nested_type})"),
        format!("RETURN NULL IS TYPED {nested_type}"),
        format!("RETURN $x::{nested_type}"),
        format!("LET VALUE x {nested_type} = NULL RETURN x"),
        format!("SESSION SET VALUE $x {nested_type} = NULL"),
        format!("CREATE NODE TYPE :T (p {nested_type})"),
    ] {
        parse(&source)
            .unwrap_or_else(|error| panic!("record type declaration: {source}: {error:?}"));
    }
    parse(&format!(
        "RETURN {}VALUE {{LET VALUE VALUE {{x INT}} = NULL RETURN VALUE}}{}",
        "0 IN [".repeat(7),
        "]".repeat(7)
    ))
    .expect("a declared binding named VALUE is not a query opener");
}

#[test]
fn quoted_type_fields_keep_literal_backslashes_at_the_retry_boundary() {
    for quote in ['"', '`'] {
        // Identifiers treat the backslash literally and double the delimiter.
        let name = format!("{quote}x\\{quote}{quote} VALUE {{RETURN 0}}{quote}");
        for expr in [
            format!("CAST(NULL AS {{{name} INT}})"),
            format!("NULL IS TYPED {{{name} INT}}"),
            format!("CAST(NULL AS {{{name} INT, VALUE {{x INT}}, other STRING}})"),
        ] {
            let body = format!("RETURN {expr}");
            let source = format!(
                "RETURN {}VALUE {{{body}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            );
            parse(&source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            // The real VALUE after the quoted field still opens frame nine.
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{body}, VALUE {{RETURN 0}}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            ));
        }
    }
}

#[test]
fn quoted_names_do_not_change_neighboring_expression_string_rules() {
    for quote in ['"', '`'] {
        let name = format!("{quote}x\\{quote}{quote} VALUE {{RETURN 0}}{quote}");
        // A single delimiter after the backslash is escaped in a string.
        // Applying identifier rules here would expose another phantom VALUE.
        let string = format!("{quote}x\\{quote} VALUE {{RETURN 0}}{quote}");
        for body in [
            format!("RETURN CAST(NULL AS {{{name} INT}}), {string}"),
            format!("RETURN {{{name}: {string}}}, {string}"),
            format!("RETURN n.{name}, {string}"),
            format!("RETURN {string} AS {name}"),
            format!("LET {name} = {string} RETURN {string}"),
            format!("LET VALUE {name} STRING = {string} RETURN {string}"),
            format!("CALL p() YIELD {name} RETURN {string}"),
        ] {
            let source = format!(
                "RETURN {}VALUE {{{body}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            );
            parse(&source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{body}, VALUE {{RETURN 0}}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            ));
        }
        parse(&format!(
            "CREATE NODE TYPE :T ({name} STRING DEFAULT {string}, other INT)"
        ))
        .expect("a quoted declaration name and its default use distinct quote rules");
    }
}

#[test]
fn quoted_scope_names_and_typed_use_resume_query_tracking_after_the_token() {
    for quote in ['"', '`'] {
        let name = format!("{quote}x\\{quote}{quote} VALUE {{RETURN 0}}{quote}");
        for (scope, unsupported) in [
            (
                format!("USE $g::RECORD {{{name} INT, VALUE {{x INT}}}}"),
                true,
            ),
            (format!("USE {name}"), false),
            (format!("AT /{name}"), false),
            (format!("CALL ({name})"), false),
        ] {
            let source = format!(
                "RETURN {}VALUE {{{scope} {{RETURN 0}}}}{}",
                "0 IN [".repeat(6),
                "]".repeat(6)
            );
            if unsupported {
                assert_scope_unsupported(&source);
            } else {
                parse(&source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            }
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{scope} {{RETURN VALUE {{RETURN 0}}}}}}{}",
                "0 IN [".repeat(6),
                "]".repeat(6)
            ));
        }
    }
}

#[test]
fn query_identifier_lists_and_patterns_preserve_quoted_name_boundaries() {
    for quote in ['"', '`'] {
        let name = format!("{quote}x\\{quote}{quote} VALUE {{RETURN 0}}{quote}");
        let string = format!("{quote}x\\{quote} VALUE {{RETURN 0}}{quote}");
        for body in [
            format!("CALL p() YIELD a, {name} RETURN {string}"),
            format!("CALL p() YIELD a AS x, {name} AS {name}, z RETURN [{string}, {string}]"),
            format!("CALL {name}() RETURN {string}"),
            format!("CALL {name}.p({string}, [{string}]) RETURN {string}"),
            format!("CALL p.{name}({string}, abs(0)) RETURN {string}"),
            format!("MATCH ({name}) RETURN {string}"),
            format!("MATCH ()-[{name}]->() RETURN {string}"),
            format!("MATCH (n:{name}) RETURN {string}"),
            format!("MATCH {name} = (n), other = ({name}) RETURN {string}"),
            format!("MATCH ANY SHORTEST {name} = (n:{name}|Other) RETURN {string}"),
            format!("MATCH (n:A&{name}:{name})<-[e:{name}]-(m) RETURN {string}"),
            format!("MATCH ({name} {{x: {string}}} WHERE {string} = {string}) RETURN {string}"),
            format!(
                "MATCH (n)-[{name}:{name} {{x: {string}}} WHERE {string} = {string}]->() RETURN {string}"
            ),
            format!("MATCH (n) WHERE {string} = {string} RETURN [{string}, {string}]"),
            format!("RETURN EXISTS {{({name}:{name})}}, {string}"),
            format!("RETURN EXISTS (({name})-[{name}:{name}]->()), {string}"),
            format!("FOR {name} IN items RETURN {string}"),
            format!("FOR n IN items WITH ORDINALITY {name} RETURN {string}"),
            format!("FOR n IN items WITH OFFSET {name} RETURN {string}"),
            format!("FOR n IN ({string}) WITH {string} AS {name} RETURN {string}"),
        ] {
            let source = format!(
                "RETURN {}VALUE {{{body}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            );
            parse(&source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{body}, VALUE {{RETURN 0}}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            ));
        }
    }
}

#[test]
fn scoped_name_state_stays_out_of_call_arguments_and_focused_mutation_values() {
    for quote in ['"', '`'] {
        let name = format!("{quote}x\\{quote}{quote} VALUE {{RETURN 0}}{quote}");
        let string = format!("{quote}x\\{quote} VALUE {{RETURN 0}}{quote}");
        for body in [
            format!("CALL (a, {name}, b) {{RETURN {string}}} RETURN [{string}, {string}]"),
            format!("USE g INSERT ({name}:{name} {{x: {string}}}) RETURN {string}"),
            format!("USE g MATCH (n) SET n.x = {string}, {name}.x = {string} RETURN {string}"),
            format!("USE g MATCH (n) SET n IS {name}, {name}:{name} RETURN {string}"),
            format!("USE g MATCH (n) REMOVE n.x, {name}.x, n IS {name} RETURN {string}"),
            format!("USE g MATCH (n) DELETE n, {name} RETURN {string}"),
            format!("USE g MERGE ({name}:{name}) ON MATCH SET {name}.x = {string} RETURN {string}"),
        ] {
            // Only braced CALL adds a retry frame. Unbraced USE mutations do not.
            let lists = if body.starts_with("CALL") { 6 } else { 7 };
            let source = format!(
                "RETURN {}VALUE {{{body}}}{}",
                "0 IN [".repeat(lists),
                "]".repeat(lists)
            );
            if body.starts_with("CALL") {
                parse(&source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            } else {
                assert_scope_unsupported(&source);
            }
            let nested = body.replacen(&format!("RETURN {string}"), "RETURN VALUE {RETURN 0}", 1);
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{nested}}}{}",
                "0 IN [".repeat(lists),
                "]".repeat(lists)
            ));
        }
        for body in [
            format!("CALL p(TABLE {name}, {string}) RETURN {string}"),
            format!("CALL p(GRAPH {name}, {string}) RETURN {string}"),
        ] {
            let source = format!(
                "RETURN {}VALUE {{{body}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            );
            let expected = parse(&body).unwrap_err();
            let actual = parse(&source).unwrap_err();
            assert!(
                matches!(actual, ParserError::UnsupportedFeature { .. }),
                "{actual:?}"
            );
            assert_eq!(actual.gqlstatus(), expected.gqlstatus());
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{body}, VALUE {{RETURN 0}}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            ));
        }
    }
}

#[test]
fn call_and_use_query_entries_share_the_retry_budget() {
    for prefix in [
        "CALL {",
        "CALL (VALUE) {",
        "USE g {",
        "USE ./g {",
        "USE /s/g {",
        "USE \"g\" {",
        "USE CURRENT_GRAPH {",
        "cAlL /* { */ () // {\n {",
    ] {
        assert_rejected_pre_pest(&format!("{}0", prefix.repeat(9)));
        assert_rejected_pre_pest(&format!("{}RETURN 0{}", prefix.repeat(9), "}".repeat(9)));
        parse(&format!("{}RETURN 0{}", prefix.repeat(8), "}".repeat(8)))
            .unwrap_or_else(|error| panic!("eight {prefix} frames: {error:?}"));
    }

    let prefix = "RETURN VALUE { CALL { USE g {";
    assert_rejected_pre_pest(&format!("{}0", prefix.repeat(3)));
    let mixed = format!(
        "{}RETURN {}0{}{}",
        prefix.repeat(2),
        "0 IN [".repeat(3),
        "]".repeat(3),
        "}".repeat(6)
    );
    assert_rejected_pre_pest(&mixed);
    let mixed = format!(
        "{}RETURN {}0{}{}",
        prefix.repeat(2),
        "0 IN [".repeat(2),
        "]".repeat(2),
        "}".repeat(6)
    );
    parse(&mixed).expect("mixed VALUE/CALL/USE/IN entries admit exactly eight frames");
    let siblings = "CALL {RETURN 0} ".repeat(20);
    parse(&format!("{siblings}RETURN 1")).expect("closed CALL siblings release their frames");
}

#[test]
fn comparison_rhs_prefixes_and_for_statements_retain_their_language() {
    for (prefix, suffix) in [("0 IN -[", "]"), ("0 IN ([", "])"), ("0 NOT IN (", ")")] {
        let source = format!("RETURN {}0{}", prefix.repeat(9), suffix.repeat(9));
        parse(&source).expect("comparison RHS prefixes do not retry the direct-list alternative");
    }
    let statements = "FOR x IN [0] ".repeat(20);
    parse(&format!("{statements}RETURN x")).expect("closed FOR lists remain admitted");
    parse(&wrap_value_queries(2, "FOR x IN [0] RETURN x"))
        .expect("FOR statements remain admitted inside ordinary VALUE queries");
}

#[test]
fn schema_property_named_value_does_not_open_query_frames() {
    let nested_type = format!("{}{{x INT}}{}", "{VALUE ".repeat(8), "}".repeat(8));
    for source in [
        format!("CREATE NODE TYPE :T (VALUE {nested_type})"),
        format!("ALTER NODE TYPE :T (x INT, VALUE {nested_type})"),
        format!("CREATE EDGE TYPE :E (FROM :N TO :N, VALUE {nested_type})"),
        format!("CREATE EDGE TYPE :E (FROM :N TO :N VALUE {nested_type})"),
        format!("CREATE NODE TYPE :T (DEFAULT {nested_type})"),
    ] {
        parse(&source).unwrap_or_else(|error| panic!("type-only declaration: {source}: {error:?}"));
    }
}

#[test]
fn comma_tail_let_declarations_do_not_open_query_frames() {
    for bindings in [
        "x = 0, VALUE VALUE {x INT} = NULL",
        "VALUE x INT = 0, VALUE VALUE LIST<{RETURN LIST}> = NULL",
        "x = [0, {a: 1}], vAlUe /* name */ VALUE TYPED {x INT} = NULL",
        "x = abs(0), VALUE VALUE {x INT} = NULL, VALUE y {VALUE {x INT}} = NULL",
    ] {
        parse(&format!(
            "RETURN {}VALUE {{LET {bindings} RETURN VALUE}}{}",
            "0 IN [".repeat(7),
            "]".repeat(7)
        ))
        .expect("a comma-tail declaration name is not a VALUE-query opener");
    }
    for body in [
        "LET x = [0, VALUE {RETURN 0}] RETURN x",
        "LET VALUE x {VALUE {x INT}} = VALUE {RETURN 0} RETURN x",
        "LET x = 0 RETURN 1, VALUE {RETURN 0}",
    ] {
        assert_rejected_pre_pest(&wrap_value_queries(8, body));
    }
}

#[test]
fn at_query_entries_count_actual_open_transitions_and_release_them() {
    for prefix in ["AT /s {", "aT /* { */ /s/t // {\n {", "AT /\"VALUE\" {"] {
        parse(&format!("{}RETURN 0{}", prefix.repeat(8), "}".repeat(8)))
            .expect("eight AT-ready query entries remain admitted");
        assert_rejected_pre_pest(&format!("{}RETURN 0{}", prefix.repeat(9), "}".repeat(9)));
        assert_rejected_pre_pest(&format!("{}0", prefix.repeat(9)));
    }
    // An initial bare brace is not a retry transition. The next nine are.
    assert_rejected_pre_pest(&format!("{{{}0", "AT /s {".repeat(9)));
    parse(&format!(
        "{{{}RETURN 0{}",
        "AT /s {".repeat(8),
        "}".repeat(9)
    ))
    .expect("the root brace does not consume the shared retry budget");

    let siblings = "CALL {AT /s {RETURN 0}} ".repeat(20);
    parse(&format!("{siblings}RETURN 1"))
        .expect("closed CALL/AT siblings each release their own two frames");
    assert_rejected_pre_pest(&wrap_value_queries(7, "CALL {AT /s {RETURN 0}} RETURN 1"));
}

fn assert_scope_unsupported(source: &str) {
    let error = parse(source).expect_err("typed USE retains its existing unsupported diagnostic");
    assert!(
        matches!(error, ParserError::NotImplemented { .. }),
        "{source}: {error:?}"
    );
    assert_eq!(error.gqlstatus().as_str(), "42N01");
}

#[test]
fn typed_use_preserves_complete_type_boundaries_and_diagnostics() {
    for value_type in [
        "GRAPH",
        "INT",
        "ANY PROPERTY GRAPH",
        "ANY RECORD",
        "RECORD",
        "RECORD {VALUE {RETURN LIST}}",
        "{VALUE {x INT}, other STRING(8)}",
        "LIST<{VALUE INT}>[4] NOT NULL",
        "INT NOT NULL ARRAY[4] LIST[8] NOT NULL",
        "ANY VALUE<INT | LIST<RECORD {x INT}>>",
        "BINDING TABLE {VALUE {x INT}, y DECIMAL(10,2)}",
        "TIMESTAMP WITH TIME ZONE",
        "SIGNED INTEGER128",
        "DURATION(DAY TO SECOND)",
        "LIST /* type */ < { `VALUE` :: LIST<INT> } > // brace\n",
    ] {
        let prefix = format!("USE $g::{value_type} {{");
        assert_scope_unsupported(&format!("{prefix}RETURN 0}}"));
        assert_scope_unsupported(&format!(
            "RETURN {}VALUE {{{prefix}RETURN 0}}}}{}",
            "0 IN [".repeat(6),
            "]".repeat(6)
        ));
        assert_rejected_pre_pest(&format!("{}0", prefix.repeat(9)));
        assert_rejected_pre_pest(&format!("{}RETURN 0{}", prefix.repeat(9), "}".repeat(9)));
    }
    let prefixes = [
        "USE $g::GRAPH {",
        "USE $g::INT {",
        "USE $g::RECORD {x INT} {",
    ];
    let nested = prefixes.into_iter().cycle().take(8).collect::<String>();
    assert_scope_unsupported(&format!("{nested}RETURN 0{}", "}".repeat(8)));
    // RECORD's optional field-type body fails here; this brace belongs to the
    // query, even though its first two tokens also resemble a record field.
    let prefix = "USE $g::RECORD { RETURN LIST + VALUE {";
    assert_rejected_pre_pest(&format!("{}RETURN 0{}", prefix.repeat(5), "}}".repeat(5)));

    let siblings = "CALL {USE $g::LIST<{VALUE INT}>[4] {RETURN 0}} ".repeat(20);
    assert_scope_unsupported(&format!("{siblings}RETURN 1"));
    let mixed = "RETURN VALUE { CALL { AT /s { USE $g::INT {";
    assert_scope_unsupported(&format!("{}RETURN 0{}", mixed.repeat(2), "}".repeat(8)));
    assert_rejected_pre_pest(&format!(
        "{}RETURN 0 IN [0]{}",
        mixed.repeat(2),
        "}".repeat(8)
    ));
}

#[test]
fn malformed_unicode_in_typed_use_is_an_error_not_a_panic() {
    for source in [
        "USE $g::😊 {RETURN 0}",
        "USE $g::{😊 INT} {RETURN 0}",
        "USE $g::INT😊 {RETURN 0}",
        "USE $g::LIST<\u{2003}INT> {RETURN 0}",
    ] {
        let error = parse(source).expect_err("malformed Unicode must return a structured error");
        assert_eq!(error.gqlstatus().as_str(), "42001");
    }
    for source in [
        "USE $グラフ::{é INT, \"😊\" LIST<INT>} {RETURN 0}",
        "USE $g::RECORD {`字``😊` STRING(8)} {RETURN 0}",
    ] {
        assert_scope_unsupported(source);
        for (end, _) in source.char_indices() {
            let error = parse(&source[..end]).expect_err("a truncated typed scope cannot succeed");
            assert!(matches!(error.gqlstatus().as_str(), "42001" | "42N01"));
        }
    }
}

fn optional_record_query(depth: usize, leaf: &str, trailing_comma: bool) -> String {
    format!(
        "USE $g::RECORD {{{}RETURN {leaf}{}{}}}",
        "CALL {".repeat(depth - 1),
        "}".repeat(depth - 1),
        if trailing_comma { "," } else { "" }
    )
}

#[test]
fn optional_record_type_mismatches_do_not_hide_query_frames() {
    for leaf in [
        "JSON(0)",
        "DOUBLE INT",
        "SIGNED INT",
        "LIST[x]",
        "LIST[1,2]",
        "ANY VALUE GRAPH",
        "INT | ANY<INT>",
        "ANY<ANY<INT>>",
        "ANY<INT> ARRAY",
    ] {
        assert_rejected_pre_pest(&optional_record_query(9, leaf, false));
    }
    assert_scope_unsupported(&optional_record_query(8, "JSON(0)", false));
    assert_rejected_pre_pest(&optional_record_query(9, "NULL", true));
}

#[test]
fn typed_use_lookahead_capacity_fails_closed() {
    for (prefix, suffix, admitted) in [("LIST<", ">", 64), ("ANY<INT | LIST<", ">>", 32)] {
        let source = format!(
            "USE $g::{}GRAPH{} {{RETURN 0}}",
            prefix.repeat(128),
            suffix.repeat(128)
        );
        let error = parse(&source).unwrap_err();
        assert!(
            matches!(error, ParserError::ComplexityLimitExceeded { .. }),
            "{error:?}"
        );
        assert_eq!(error.gqlstatus(), GqlStatus::PROGRAM_LIMIT_EXCEEDED);
        assert_scope_unsupported(&format!(
            "USE $g::{}GRAPH{} {{RETURN 0}}",
            prefix.repeat(admitted),
            suffix.repeat(admitted)
        ));
    }
}
