//! Preserve the grammar's complete function-call/literal quote decision.

use selene_gql::parse;

use super::query_reentry::assert_rejected_pre_pest;

#[rstest::rstest]
#[case('"', "")]
#[case('`', "")]
#[case('"', "0")]
#[case('`', "0")]
fn complete_quoted_expression_calls_keep_identifier_boundaries(
    #[case] quote: char,
    #[case] arguments: &str,
) {
    let name = format!("{quote}x\\{quote}{quote} VALUE {{RETURN 0}}{quote}");
    for call in [
        format!("{name}({arguments})"),
        format!("{name}.p /* call */ ( {arguments} )"),
    ] {
        let body = format!("RETURN {call}");
        parse(&body).expect("the expression call is accepted without wrapping frames");
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

#[test]
fn incomplete_quoted_expression_call_retains_the_valid_string_fallback() {
    for quote in ['"', '`'] {
        for tail in ["(broken", "(+)", "(0,)"] {
            // Identifier scanning sees a name followed by `(`, but the attempted
            // function call cannot finish. Pest instead parses three projections:
            // a string ending at the doubled delimiter, a real VALUE, and a string
            // beginning at what identifier scanning mistook for its closing quote.
            let fallback = format!(
                "RETURN {quote}x\\{quote}{quote}, VALUE {{RETURN 0}}, {quote}{tail}{quote}"
            );
            parse(&format!(
                "RETURN {}VALUE {{{fallback}}}{}",
                "0 IN [".repeat(6),
                "]".repeat(6)
            ))
            .expect("the valid string fallback contains exactly eight real frames");
            assert_rejected_pre_pest(&format!(
                "RETURN {}VALUE {{{fallback}}}{}",
                "0 IN [".repeat(7),
                "]".repeat(7)
            ));
        }
    }
}
