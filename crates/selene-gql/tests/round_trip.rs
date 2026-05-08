//! Public parser round-trip smoke tests.

use selene_gql::{PipelineStatement, Statement, ValueExpr, parse};

#[test]
fn canonical_read_query_builds_structural_ast() {
    let source = "\
MATCH (n:Person)
WHERE n.age > 30 AND n.active = true
RETURN n.name AS name, n.age AS age
ORDER BY n.age DESC, n.name ASC
LIMIT 10 OFFSET 5";

    let Statement::Query(pipeline) = parse(source).expect("canonical read query parses") else {
        panic!("expected read query pipeline");
    };

    assert_eq!(pipeline.span.byte_offset, 0);
    assert_eq!(pipeline.span.byte_len as usize, source.len());
    assert_eq!(pipeline.statements.len(), 5);

    let PipelineStatement::Match(match_clause) = &pipeline.statements[0] else {
        panic!("expected MATCH");
    };
    assert!(match_clause.where_clause.is_some());
    assert_eq!(match_clause.patterns.len(), 1);

    let PipelineStatement::Return(return_clause) = &pipeline.statements[1] else {
        panic!("expected RETURN");
    };
    assert_eq!(return_clause.items.len(), 2);
    assert!(matches!(
        return_clause.items[0].expr,
        ValueExpr::PropertyAccess { .. }
    ));

    assert!(matches!(
        pipeline.statements[2],
        PipelineStatement::Sorting(_)
    ));
    assert!(matches!(
        pipeline.statements[3],
        PipelineStatement::Limit(_)
    ));
    assert!(matches!(
        pipeline.statements[4],
        PipelineStatement::Offset(_)
    ));
}
