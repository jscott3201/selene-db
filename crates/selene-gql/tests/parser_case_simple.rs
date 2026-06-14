//! Parser coverage for simple CASE expression lowering.

use selene_gql::{BinaryOp, PipelineStatement, Statement, ValueExpr, parse};

fn only_item(source: &str) -> selene_gql::ReturnItem {
    let Statement::Query(query) = parse(source).expect("parse succeeds") else {
        panic!("expected query statement");
    };
    assert_eq!(query.statements.len(), 1);
    let PipelineStatement::Return(clause) = query.statements.into_iter().next().unwrap() else {
        panic!("expected return clause");
    };
    assert_eq!(clause.items.len(), 1);
    clause.items.into_iter().next().unwrap()
}

#[test]
fn parse_simple_case_when_operand_list() {
    let item = only_item("RETURN CASE 2 WHEN 1, 2 THEN 'hit' ELSE 'miss' END");
    let ValueExpr::Case { ref branches, .. } = item.expr else {
        panic!("expected CASE expression");
    };
    assert_eq!(branches.len(), 1);
    let (condition, _) = &branches[0];
    let ValueExpr::BinaryOp {
        op: BinaryOp::Or,
        lhs,
        rhs,
        ..
    } = condition
    else {
        panic!("expected operand-list CASE to lower to OR, got {condition:?}");
    };
    assert!(matches!(
        lhs.as_ref(),
        ValueExpr::BinaryOp {
            op: BinaryOp::Eq,
            ..
        }
    ));
    assert!(matches!(
        rhs.as_ref(),
        ValueExpr::BinaryOp {
            op: BinaryOp::Eq,
            ..
        }
    ));
}
