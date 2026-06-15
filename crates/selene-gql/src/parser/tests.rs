use super::*;
use crate::ast::{
    BinaryOp, BindingTableType, CharacterStringLiteralKind, EdgeDirection, GqlType,
    IntegerLiteralKind, IsCheckKind, LabelExpr, Literal, PipelineStatement,
    RowExpansionPositionKind, SetOp, ValueExpr,
};
use crate::error::GqlStatus;

mod batch;
mod expressions;
mod literals;
mod numeric;
mod patterns;
mod radix;
mod rejections;
mod statements;

fn query(source: &str) -> crate::ast::QueryPipeline {
    let Statement::Query(query) = parse(source).expect("parse succeeds") else {
        panic!("expected query statement");
    };
    query
}

fn parse_unflagged(source: &str) -> Statement {
    guard::validate(source).expect("source passes parser guard");
    let mut pairs = GqlParser::parse(Rule::gql_program, source).expect("pest parse succeeds");
    let program_pair = pairs.next().expect("program pair exists");
    builders::build_statement(program_pair).expect("AST build succeeds")
}

fn return_clause(source: &str) -> crate::ast::ReturnClause {
    let query = query(source);
    assert_eq!(query.statements.len(), 1);
    let PipelineStatement::Return(clause) = query.statements.into_iter().next().unwrap() else {
        panic!("expected return clause");
    };
    clause
}

fn only_item(source: &str) -> crate::ast::ReturnItem {
    let clause = return_clause(source);
    assert_eq!(clause.items.len(), 1);
    clause.items.into_iter().next().unwrap()
}

fn only_unflagged_item(source: &str) -> crate::ast::ReturnItem {
    let Statement::Query(query) = parse_unflagged(source) else {
        panic!("expected query statement");
    };
    assert_eq!(query.statements.len(), 1);
    let PipelineStatement::Return(clause) = query.statements.into_iter().next().unwrap() else {
        panic!("expected return clause");
    };
    assert_eq!(clause.items.len(), 1);
    clause.items.into_iter().next().unwrap()
}

fn optional_name(value: Option<selene_core::DbString>) -> Option<String> {
    value.map(|name| name.as_str().to_owned())
}

fn assert_function_call(source: &str, expected_name: &str) {
    assert_function_call_with_args(source, expected_name, 0);
}

fn assert_function_call_with_args(source: &str, expected_name: &str, expected_args: usize) {
    let expr = only_item(source).expr;
    let ValueExpr::FunctionCall {
        ref name,
        ref args,
        star,
        distinct,
        ..
    } = expr
    else {
        panic!("expected function call expression for {source}");
    };
    assert_eq!(name.len(), 1, "{source}");
    assert_eq!(name.first().as_str(), expected_name, "{source}");
    assert_eq!(args.len(), expected_args, "{source}");
    assert!(!star, "{source}");
    assert!(!distinct, "{source}");
}

// PARSER-DOS complexity-guard unit tests live in the dedicated integration
// suite `tests/parser_dos_artifacts.rs` (the `dos` regression module),
// alongside the embedded fuzz artifacts, so this near-cap module file stays
// comfortably under the 700-LOC gate. The guard maps to GQLSTATUS 5GQL1
// PROGRAM_LIMIT_EXCEEDED per ISO/IEC 39075:2024 section 23.1; see
// `crate::error::GqlStatus::PROGRAM_LIMIT_EXCEEDED` for the Table 8 grounding.
