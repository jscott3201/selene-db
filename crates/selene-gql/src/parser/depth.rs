//! Post-build expression-depth guard.
//!
//! pest parses flat left-associative operator folds (`a OR a OR …`,
//! `1 - 1 - 1 - …`) and postfix chains (`a.b.c.…`) *iteratively* (a star-loop
//! in the grammar, a `while`/`for` loop in the builders — see
//! `builders::expr`), so neither pest's descent nor AST construction overflows
//! on them. But the result is a depth-N left-leaning `Box<ValueExpr>` tree, and
//! the *recursive* consumers of that tree — the Flagger walk that runs inside
//! [`parse`](super::parse), the analyzer/planner/optimizer/formatter that run
//! after it, and the derived `Drop`/`Clone`/`PartialEq` glue — recurse one
//! native frame per level. Past ~130k levels that overflows the native stack, a
//! **non-unwindable** crash (`catch_unwind` cannot trap it) that hard-kills the
//! host process embedding selene-db.
//!
//! This guard runs once, after [`build_statement`](super::builders::build_statement)
//! and before the Flagger, and rejects any expression whose `Box<ValueExpr>`
//! nesting depth exceeds [`MAX_EXPR_DEPTH`], mapped to
//! [`ParserError::ComplexityLimitExceeded`] → GQLSTATUS `5GQL1`. It is itself
//! **iterative** (an explicit worklist, never recursion), so it cannot overflow
//! on the very input it rejects, and it is **statement-wide**: it reaches every
//! expression in every clause and descends subquery bodies (`EXISTS`/`COUNT`/
//! `VALUE { … }`), because the Flagger walks those too. Expression-child
//! enumeration delegates to the centralized [`ValueExpr::for_each_child`] so the
//! per-variant child shape lives in exactly one place (`ast::walk`).
//!
//! The cap is the *same* 256 as the analyzer's `ANALYZER_MAX_DEPTH` and the
//! pre-pest guard's `MAX_RECURSION_DEPTH`: the analyzer already rejects any
//! expression deeper than that with `RecursionLimitExceeded` before binding, so
//! capping the parser at 256 newly rejects **zero** executable queries — it only
//! converts a later analyzer rejection into an earlier, equivalent parser
//! rejection and closes the in-parse Flagger / `Drop` crash. The deepest
//! legitimate expression nesting anywhere in the workspace is 3.

use crate::{
    ast::{
        call::{InlineProcedureCall, ProcedureCall},
        ddl::{DdlStatement, TypePropertyConstraint},
        expr::{IsCheckKind, ValueExpr},
        mutation::{MutationPipeline, MutationStatement, MutationTerminator, SetItem},
        pattern::{GraphPattern, MatchClause, PatternElement},
        statement::{PipelineStatement, QueryPipeline, ReturnClause, Statement, WithClause},
    },
    error::ParserError,
};

/// Maximum admitted `Box<ValueExpr>` nesting depth of any single expression.
///
/// Mirrors `analyze::bind::ANALYZER_MAX_DEPTH` and `parser::guard::MAX_RECURSION_DEPTH`
/// (all 256) so the parser, the pre-pest guard, and the analyzer share one
/// recursion ceiling. See the module docs for why this cap is *free* (the
/// analyzer rejects deeper expressions regardless).
pub(crate) const MAX_EXPR_DEPTH: u32 = 256;

/// One unit of pending traversal work.
///
/// The guard walks the whole statement with an explicit stack of these so it
/// never recurses (and so cannot itself overflow on a hostile-depth tree).
/// `Expr` items carry the running `Box<ValueExpr>` nesting depth; every other
/// variant is a structural container whose expression-bearing fields are pushed
/// as fresh `Expr(_, 1)` roots (a subquery boundary starts a new expression
/// context, so its depth resets).
enum Node<'a> {
    Statement(&'a Statement),
    Pipeline(&'a QueryPipeline),
    Mutation(&'a MutationPipeline),
    MatchClause(&'a MatchClause),
    Pattern(&'a GraphPattern),
    Return(&'a ReturnClause),
    With(&'a WithClause),
    Call(&'a ProcedureCall),
    Ddl(&'a DdlStatement),
    Expr(&'a ValueExpr, u32),
}

/// Reject any expression in `statement` whose nesting depth exceeds
/// [`MAX_EXPR_DEPTH`].
///
/// # Errors
///
/// Returns [`ParserError::ComplexityLimitExceeded`] (GQLSTATUS `5GQL1`) at the
/// span of the first node that crosses the cap.
pub(super) fn reject_excessive_expr_depth(statement: &Statement) -> Result<(), ParserError> {
    let mut work = vec![Node::Statement(statement)];
    while let Some(node) = work.pop() {
        match node {
            Node::Statement(statement) => push_statement(statement, &mut work),
            Node::Pipeline(pipeline) => push_pipeline(pipeline, &mut work),
            Node::Mutation(pipeline) => push_mutation(pipeline, &mut work),
            Node::MatchClause(clause) => push_match_clause(clause, &mut work),
            Node::Pattern(pattern) => push_pattern(pattern, &mut work),
            Node::Return(clause) => push_return(clause, &mut work),
            Node::With(clause) => push_with(clause, &mut work),
            Node::Call(call) => push_call(call, &mut work),
            Node::Ddl(statement) => push_ddl(statement, &mut work),
            Node::Expr(expr, depth) => push_expr(expr, depth, &mut work)?,
        }
    }
    Ok(())
}

fn push_expr<'a>(
    expr: &'a ValueExpr,
    depth: u32,
    work: &mut Vec<Node<'a>>,
) -> Result<(), ParserError> {
    if depth > MAX_EXPR_DEPTH {
        return Err(ParserError::ComplexityLimitExceeded {
            limit: MAX_EXPR_DEPTH,
            span: expr.span(),
        });
    }
    let next = depth.saturating_add(1);
    // Enqueue every child to walk next. `ValueExpr::for_each_child` hands out
    // children with a higher-ranked lifetime that cannot be stored on the
    // worklist, so the child enumeration is matched manually here (mirroring the
    // analyzer's `check_expr_depth`); the match is exhaustive — no wildcard — so
    // a future variant with children or a subquery body forces an update rather
    // than silently escaping the depth bound. `Cast`'s `target_type`
    // (`Box<GqlType>`) is intentionally not enqueued: `GqlType` nesting is
    // bounded by the type builder's depth gate, and its consumers run on a
    // ≤64-deep tree.
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        ValueExpr::PropertyAccess { target, .. } | ValueExpr::PropertyExists { target, .. } => {
            work.push(Node::Expr(target, next));
        }
        ValueExpr::ListAccess { target, index, .. } => {
            work.push(Node::Expr(target, next));
            work.push(Node::Expr(index, next));
        }
        ValueExpr::ListLiteral { items, .. }
        | ValueExpr::AllDifferent { items, .. }
        | ValueExpr::Same { items, .. } => {
            for item in items {
                work.push(Node::Expr(item, next));
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                work.push(Node::Expr(value, next));
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            work.push(Node::Expr(lhs, next));
            work.push(Node::Expr(rhs, next));
        }
        ValueExpr::UnaryOp { operand, .. } => work.push(Node::Expr(operand, next)),
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                work.push(Node::Expr(arg, next));
            }
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            work.push(Node::Expr(operand, next));
            match kind {
                IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
                    work.push(Node::Expr(value, next));
                }
                IsCheckKind::Null
                | IsCheckKind::Directed
                | IsCheckKind::Labeled(_)
                | IsCheckKind::TruthValue(_)
                | IsCheckKind::Typed(_)
                | IsCheckKind::Normalized(_) => {}
            }
        }
        ValueExpr::InList { operand, list, .. } => {
            work.push(Node::Expr(operand, next));
            for item in list {
                work.push(Node::Expr(item, next));
            }
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, value) in branches {
                work.push(Node::Expr(condition, next));
                work.push(Node::Expr(value, next));
            }
            if let Some(value) = else_branch {
                work.push(Node::Expr(value, next));
            }
        }
        ValueExpr::Cast { value, .. } | ValueExpr::Normalize { source: value, .. } => {
            work.push(Node::Expr(value, next));
        }
        ValueExpr::Trim {
            character, source, ..
        } => {
            if let Some(character) = character {
                work.push(Node::Expr(character, next));
            }
            work.push(Node::Expr(source, next));
        }
        // Subquery bodies are `MatchClause` / `QueryPipeline`, not direct
        // `ValueExpr` children — descend into them (a fresh expression context,
        // so depth resets to 1 in the structural push helpers).
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            work.push(Node::MatchClause(pattern));
        }
        ValueExpr::ValueSubquery { body, .. } => work.push(Node::Pipeline(body)),
    }
    Ok(())
}

fn push_statement<'a>(statement: &'a Statement, work: &mut Vec<Node<'a>>) {
    match statement {
        Statement::Query(pipeline) => work.push(Node::Pipeline(pipeline)),
        Statement::Composite { first, rest, .. } => {
            work.push(Node::Pipeline(first));
            for (_, pipeline) in rest.iter() {
                work.push(Node::Pipeline(pipeline));
            }
        }
        Statement::Chained { blocks, .. } => {
            for pipeline in blocks {
                work.push(Node::Pipeline(pipeline));
            }
        }
        Statement::Mutate(pipeline) => work.push(Node::Mutation(pipeline)),
        Statement::Ddl(statement) => work.push(Node::Ddl(statement)),
        Statement::Call(call) => work.push(Node::Call(call)),
        Statement::Explain { inner, .. } => work.push(Node::Statement(inner)),
        Statement::SessionSetValue { value, .. } => work.push(Node::Expr(value, 1)),
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::SessionSetTimeZone { .. }
        | Statement::SessionReset { .. }
        | Statement::SessionClose { .. } => {}
    }
}

fn push_pipeline<'a>(pipeline: &'a QueryPipeline, work: &mut Vec<Node<'a>>) {
    for statement in &pipeline.statements {
        match statement {
            PipelineStatement::Match(clause) => work.push(Node::MatchClause(clause)),
            PipelineStatement::Filter(expr) => work.push(Node::Expr(expr, 1)),
            PipelineStatement::Let(bindings) => {
                for binding in bindings {
                    work.push(Node::Expr(&binding.value, 1));
                }
            }
            PipelineStatement::Unwind(unwind) => work.push(Node::Expr(&unwind.source, 1)),
            PipelineStatement::Sorting(terms) => {
                for term in terms {
                    work.push(Node::Expr(&term.expr, 1));
                }
            }
            PipelineStatement::Limit(_) | PipelineStatement::Offset(_) => {}
            PipelineStatement::Return(clause) => work.push(Node::Return(clause)),
            PipelineStatement::With(clause) => work.push(Node::With(clause)),
            PipelineStatement::Call(call) => work.push(Node::Call(call)),
            PipelineStatement::CallSubquery(call) => push_inline_call(call, work),
        }
    }
}

fn push_inline_call<'a>(call: &'a InlineProcedureCall, work: &mut Vec<Node<'a>>) {
    work.push(Node::Pipeline(&call.body));
}

fn push_mutation<'a>(pipeline: &'a MutationPipeline, work: &mut Vec<Node<'a>>) {
    for statement in pipeline.statements.iter() {
        match statement {
            MutationStatement::Match(clause) => work.push(Node::MatchClause(clause)),
            MutationStatement::Filter(expr) => work.push(Node::Expr(expr, 1)),
            MutationStatement::Insert(insert) => {
                for pattern in &insert.patterns {
                    work.push(Node::Pattern(pattern));
                }
            }
            MutationStatement::Set(items) => {
                for item in items {
                    push_set_item(item, work);
                }
            }
            MutationStatement::Remove(_) | MutationStatement::Delete(_) => {}
        }
    }
    if let Some(MutationTerminator::Return(clause)) = &pipeline.terminator {
        work.push(Node::Return(clause));
    }
}

fn push_set_item<'a>(item: &'a SetItem, work: &mut Vec<Node<'a>>) {
    match item {
        SetItem::Property { value, .. } => work.push(Node::Expr(value, 1)),
        SetItem::PropertyMerge { properties, .. } => {
            for (_, value) in properties {
                work.push(Node::Expr(value, 1));
            }
        }
        SetItem::Label { .. } => {}
    }
}

fn push_match_clause<'a>(clause: &'a MatchClause, work: &mut Vec<Node<'a>>) {
    for pattern in &clause.patterns {
        work.push(Node::Pattern(pattern));
    }
    if let Some(where_clause) = &clause.where_clause {
        work.push(Node::Expr(where_clause, 1));
    }
}

fn push_pattern<'a>(pattern: &'a GraphPattern, work: &mut Vec<Node<'a>>) {
    for element in &pattern.elements {
        let (properties, inline_where) = match element {
            PatternElement::Node(node) => (&node.properties, &node.inline_where),
            PatternElement::Edge(edge) => (&edge.properties, &edge.inline_where),
        };
        for (_, value) in properties {
            work.push(Node::Expr(value, 1));
        }
        if let Some(inline_where) = inline_where {
            work.push(Node::Expr(inline_where, 1));
        }
    }
}

fn push_return<'a>(clause: &'a ReturnClause, work: &mut Vec<Node<'a>>) {
    for item in &clause.items {
        work.push(Node::Expr(&item.expr, 1));
    }
    if let Some(group_by) = &clause.group_by {
        for expr in group_by {
            work.push(Node::Expr(expr, 1));
        }
    }
    if let Some(having) = &clause.having {
        work.push(Node::Expr(having, 1));
    }
}

fn push_with<'a>(clause: &'a WithClause, work: &mut Vec<Node<'a>>) {
    for item in &clause.items {
        work.push(Node::Expr(&item.expr, 1));
    }
    if let Some(group_by) = &clause.group_by {
        for expr in group_by {
            work.push(Node::Expr(expr, 1));
        }
    }
    if let Some(having) = &clause.having {
        work.push(Node::Expr(having, 1));
    }
    if let Some(where_clause) = &clause.where_clause {
        work.push(Node::Expr(where_clause, 1));
    }
}

fn push_call<'a>(call: &'a ProcedureCall, work: &mut Vec<Node<'a>>) {
    for arg in &call.args {
        work.push(Node::Expr(arg, 1));
    }
    if let Some(filter) = &call.yield_filter {
        work.push(Node::Expr(filter, 1));
    }
}

fn push_ddl<'a>(statement: &'a DdlStatement, work: &mut Vec<Node<'a>>) {
    let properties = match statement {
        DdlStatement::CreateNodeType { properties, .. }
        | DdlStatement::CreateEdgeType { properties, .. } => properties,
        DdlStatement::CreateGraph { .. }
        | DdlStatement::DropGraph { .. }
        | DdlStatement::DropNodeType { .. }
        | DdlStatement::DropEdgeType { .. }
        | DdlStatement::TruncateNodeType { .. }
        | DdlStatement::TruncateEdgeType { .. }
        | DdlStatement::CreateIndex { .. }
        | DdlStatement::DropIndex { .. }
        | DdlStatement::ShowNodeTypes(_)
        | DdlStatement::ShowEdgeTypes(_)
        | DdlStatement::ShowIndexes(_)
        | DdlStatement::ShowProcedures(_) => return,
    };
    for property in properties {
        for constraint in &property.constraints {
            if let TypePropertyConstraint::Default(value, _) = constraint {
                work.push(Node::Expr(value, 1));
            }
        }
    }
}
