//! Post-build expression-depth guard coverage (node 818, Family B).
//!
//! Flat left-associative operator folds (`a OR a OR …`) and postfix chains
//! (`a.b.c.…`) parse and build *iteratively*, but produce a depth-N
//! left-leaning `Box<ValueExpr>` tree that overflows the recursive Flagger walk
//! (inside `parse`), the recursive destructor (`Drop`), and the post-parse
//! recursive consumers. The parser bounds expression nesting depth at
//! `parser::depth::MAX_EXPR_DEPTH` (256) and a manual iterative `Drop` makes
//! tearing a rejected over-cap tree down safe. These tests assert by behavior:
//! a hostile-depth expression is rejected as `ComplexityLimitExceeded` (5GQL1)
//! rather than crashing, a legitimate moderate depth parses, and a directly
//! built deep tree drops without overflowing a small stack.

use selene_gql::{BinaryOp, GqlStatus, Literal, ParserError, SourceSpan, ValueExpr, parse};

/// Mirror of `parser::depth::MAX_EXPR_DEPTH` (not re-exported). An expression
/// whose `Box<ValueExpr>` nesting depth exceeds this is rejected pre-Flagger.
const MAX_EXPR_DEPTH: usize = 256;

/// A depth comfortably past the crash floor (~130k recursive `Drop`, ~175k
/// recursive Flagger): big enough that a recursive consumer would SIGABRT, so
/// the guard's iterative rejection is what keeps these tests from killing the
/// runner. Used only by the two dedicated scale tests (building/dropping a tree
/// this large is the slow part).
const HOSTILE_OPERATORS: usize = 200_000;

/// A depth comfortably over [`MAX_EXPR_DEPTH`] but cheap to build/drop. The
/// guard rejects a flat fold at depth 257 regardless of total length, so this
/// is enough to prove a given clause position is *reached* and rejected; the
/// two scale tests prove no-crash on a past-the-floor tree.
const REACH_OPERATORS: usize = 5_000;

fn assert_rejected_for_depth(source: &str, label: &str) {
    let error = parse(source).expect_err(label);
    assert!(
        matches!(error, ParserError::ComplexityLimitExceeded { limit, .. } if limit as usize == MAX_EXPR_DEPTH),
        "{label}: expected ComplexityLimitExceeded(limit=256), got {error:?}"
    );
    assert_eq!(
        error.gqlstatus(),
        GqlStatus::PROGRAM_LIMIT_EXCEEDED,
        "{label}"
    );
}

#[test]
fn deep_binary_folds_reject_rather_than_crash() {
    // Every left-associative fold family builds the same depth-N spine. None
    // may crash; all must reject. Build the operand string once per operator.
    for (op, build) in [
        ("OR", " OR a"),
        ("AND", " AND a"),
        ("XOR", " XOR a"),
        ("concat", " || a"),
        ("add", " + 1"),
        ("sub", " - 1"),
        ("mul", " * 1"),
        ("compare", " = 1 OR a"),
    ] {
        let mut source = if op == "compare" {
            String::from("RETURN a")
        } else if matches!(op, "add" | "sub" | "mul") {
            String::from("RETURN 1")
        } else {
            String::from("RETURN a")
        };
        for _ in 0..REACH_OPERATORS {
            source.push_str(build);
        }
        assert_rejected_for_depth(&source, op);
    }
}

#[test]
fn deep_property_access_chain_rejects() {
    // `a.b.b.b.…` is built by the iterative postfix loop into a depth-N
    // PropertyAccess spine — another non-delimiter, non-sign depth source.
    let mut source = String::from("RETURN a");
    for _ in 0..REACH_OPERATORS {
        source.push_str(".b");
    }
    assert_rejected_for_depth(&source, "property-access chain");
}

#[test]
fn deep_fold_inside_subquery_rejects() {
    // A deep fold nested inside `EXISTS { MATCH … WHERE <fold> }` is reached by
    // the recursive Flagger via the subquery body; the depth guard must descend
    // the subquery boundary to bound it.
    let mut where_expr = String::from("a");
    for _ in 0..REACH_OPERATORS {
        where_expr.push_str(" OR a");
    }
    let source = format!("MATCH (n) WHERE EXISTS {{ MATCH (a) WHERE {where_expr} }} RETURN n");
    assert_rejected_for_depth(&source, "fold inside EXISTS subquery");
}

#[test]
fn deep_fold_inside_value_subquery_rejects() {
    let mut inner = String::from("a");
    for _ in 0..REACH_OPERATORS {
        inner.push_str(" OR a");
    }
    let source = format!("RETURN VALUE {{ MATCH (a) RETURN {inner} }} AS v");
    assert_rejected_for_depth(&source, "fold inside VALUE subquery");
}

#[test]
fn deep_fold_in_mutation_set_rejects() {
    // SET values route through the same expression builder; the guard must
    // reach the mutation pipeline.
    let mut value = String::from("1");
    for _ in 0..REACH_OPERATORS {
        value.push_str(" + 1");
    }
    let source = format!("MATCH (n) SET n.x = {value} FINISH");
    assert_rejected_for_depth(&source, "fold in SET value");
}

#[test]
fn deep_fold_in_function_argument_rejects() {
    let mut arg = String::from("1");
    for _ in 0..REACH_OPERATORS {
        arg.push_str(" + 1");
    }
    let source = format!("RETURN abs({arg})");
    assert_rejected_for_depth(&source, "fold in function argument");
}

#[test]
fn deep_fold_in_list_literal_rejects() {
    let mut item = String::from("1");
    for _ in 0..REACH_OPERATORS {
        item.push_str(" + 1");
    }
    let source = format!("RETURN [{item}]");
    assert_rejected_for_depth(&source, "fold in list literal");
}

#[test]
fn deep_fold_in_let_binding_rejects() {
    let mut value = String::from("1");
    for _ in 0..REACH_OPERATORS {
        value.push_str(" + 1");
    }
    let source = format!("MATCH (n) LET x = {value} RETURN x");
    assert_rejected_for_depth(&source, "fold in LET binding");
}

#[test]
fn moderate_expression_depth_parses() {
    // A fold well under the cap (depth ~200) must still parse — the guard must
    // not regress legitimate, machine-generated moderate expressions.
    let mut source = String::from("RETURN 1");
    for _ in 0..200 {
        source.push_str(" + 1");
    }
    parse(&source).expect("a 200-operator fold parses");

    // A realistic WHERE disjunction.
    let mut where_clause = String::from("MATCH (n) WHERE n.id = 0");
    for value in 1..150 {
        where_clause.push_str(&format!(" OR n.id = {value}"));
    }
    where_clause.push_str(" RETURN n");
    parse(&where_clause).expect("a 150-term WHERE disjunction parses");
}

#[test]
fn iterative_drop_does_not_overflow_small_stack() {
    // Build a depth-200k left-leaning BinaryOp tree directly (bypassing the
    // parser cap) and drop it on a deliberately small 256 KB stack. A recursive
    // destructor overflows that stack after a few thousand levels — a
    // non-unwindable SIGABRT that would kill the whole test process — so the
    // thread completing proves the manual iterative `Drop` tears the tree down
    // with a heap worklist instead of native stack frames.
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let span = SourceSpan::new(0, 1);
            let mut expr = ValueExpr::Literal(Literal::Integer(1, span));
            for _ in 0..200_000 {
                expr = ValueExpr::BinaryOp {
                    op: BinaryOp::Or,
                    lhs: Box::new(expr),
                    rhs: Box::new(ValueExpr::Literal(Literal::Integer(1, span))),
                    span,
                };
            }
            // Explicit drop documents intent; scope-exit drop would do the same.
            drop(expr);
        })
        .expect("spawn deep-drop thread");
    handle
        .join()
        .expect("deep ValueExpr tree drops without overflowing a 256 KB stack");
}

#[test]
fn parsing_hostile_fold_then_dropping_is_safe() {
    // End-to-end: parsing a hostile fold rejects (the tree is fully built, the
    // iterative guard rejects at depth 257, and the rejected ~200k-node tree is
    // then dropped via the iterative `Drop`). This must complete on the default
    // test-thread stack without a crash.
    let mut source = String::from("RETURN a");
    for _ in 0..HOSTILE_OPERATORS {
        source.push_str(" OR a");
    }
    let error = parse(&source).expect_err("hostile fold rejects");
    assert!(matches!(error, ParserError::ComplexityLimitExceeded { .. }));
}
