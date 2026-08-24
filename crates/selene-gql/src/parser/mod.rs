//! Pest-backed GQL parser entry points.
//!
//! The parser admits one GQL program, builds the public AST with source spans
//! preserved, and runs the
//! Flagger before callers see unsupported syntax. It does not resolve names,
//! infer types, or choose execution behavior; those invariants start at the
//! analyzer. Deferred grammar surfaces return `ParserError::NotImplemented`
//! with D1 support guidance. See ISO GQL Clause 14 and Spec 07.

mod builders;
mod depth;
mod guard;
mod many;

use std::sync::Arc;

use pest::{Parser, error::InputLocation};

use crate::{
    ast::{SourceSpan, Statement},
    diagnostic::DiagnosticReport,
    error::ParserError,
    flagger,
};

use self::pest_impl::{GqlParser, Rule};

mod pest_impl {
    #![allow(missing_docs)]

    #[derive(pest_derive::Parser)]
    #[grammar = "parser/grammar.pest"]
    pub(crate) struct GqlParser;
}

pub(crate) const MAX_NESTING_DEPTH: u32 = guard::MAX_NESTING_DEPTH;

/// Parse one GQL program.
///
/// # Errors
///
/// Returns [`ParserError::SyntaxError`] for parse failures and
/// [`ParserError::NotImplemented`] for grammar surfaces whose AST builders
/// are not yet supported. [`ParserError::UnsupportedFeature`] reports parsed
/// capabilities rejected by the generated profile's Flagger disposition.
#[tracing::instrument(name = "selene.gql.parse", skip(source), fields(source_len = source.len()))]
pub fn parse(source: &str) -> Result<Statement, ParserError> {
    guard::validate(source)?;
    // Why: pest's generated recursive descent and the AST builder both recurse
    // one native stack frame per expression-nesting level. `guard::validate`
    // deterministically bounds the *known* zero-delimiter recursion drivers
    // (unary signs, `NOT`, `CASE`) to `MAX_RECURSION_DEPTH = 256`, but pest
    // cannot re-invoke `maybe_grow` per generated frame, so this single big
    // segment is the floor any *un*enumerated future driver runs on. A stack
    // overflow in pest is non-unwindable and would hard-kill the host process,
    // so we run the descent + builder on a generous 32 MB segment via
    // `stacker::maybe_grow`. The 24 MB red zone forces the grow so the work
    // always runs on the large segment regardless of the caller's remaining
    // stack (an embedder may call `parse` from an arbitrarily deep frame).
    // `stacker` runs the closure on the same logical thread, preserving the
    // `&str` borrow, thread-locals, and panic propagation (a spawned worker
    // would not); it caches the segment thread-locally, so the warm path adds
    // near-zero overhead. Precedent: `analyze/bind/expr.rs`,
    // `runtime/evaluator/cast.rs`.
    stacker::maybe_grow(PARSE_STACK_RED_ZONE, PARSE_STACK_SEGMENT, || {
        let mut pairs = GqlParser::parse(Rule::gql_program, source)
            .map_err(|error| pest_error(source, error))?;
        let program_pair = pairs.next().ok_or_else(ParserError::empty_program)?;
        let statement = builders::build_statement(program_pair)?;
        // Bound expression nesting depth before the recursive Flagger walk (and
        // before handing the AST to any other recursive consumer). pest and the
        // builders fold flat operator chains (`a OR a OR …`) and postfix chains
        // (`a.b.c.…`) iteratively, so they do not overflow on them — but the
        // resulting depth-N `Box<ValueExpr>` tree overflows the recursive
        // Flagger / `Drop` / analyzer at ~130k deep (a non-unwindable crash).
        // `depth::reject_excessive_expr_depth` is itself iterative, so it cannot
        // overflow on the input it rejects; the manual iterative `Drop` on
        // `ValueExpr` makes tearing the rejected over-cap tree down safe.
        depth::reject_excessive_expr_depth(&statement)?;
        flagger::flag(&statement)?;
        Ok(statement)
    })
}

/// Return whether `name` is the decoded spelling of a GQL parameter name.
///
/// Names omit the leading `$`. This uses the parser's `param_ref` rule, so
/// Unicode letters and numbers, underscores, and keyword spellings stay in
/// lockstep with source parsing.
#[must_use]
pub fn is_parameter_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('$') {
        return false;
    }
    let source = format!("${name}");
    GqlParser::parse(Rule::param_ref, &source)
        .ok()
        .and_then(|mut pairs| pairs.next())
        .is_some_and(|pair| pair.as_str().len() == source.len())
}

/// Red-zone for the parser's [`stacker::maybe_grow`] backstop.
///
/// Sized larger than [`PARSE_STACK_SEGMENT`] minus a small margin so the grow
/// always fires on entry — pest's generated descent cannot itself call
/// `maybe_grow`, so the whole parse must run on a freshly grown segment, never
/// the caller's (unknown, possibly nearly exhausted) stack.
const PARSE_STACK_RED_ZONE: usize = 24 * 1024 * 1024;

/// Stack segment size for the parser's [`stacker::maybe_grow`] backstop.
///
/// 32 MB is generous relative to the analyzer's 1 MB segment because pest's
/// generated recursive descent runs entirely within this single segment (it
/// cannot grow per frame). It raises the overflow floor for any unenumerated
/// recursion driver to an implausibly large, fuzz-catchable input; the
/// deterministic [`guard::validate`] cap is the guarantee for the known ones.
const PARSE_STACK_SEGMENT: usize = 32 * 1024 * 1024;

/// Parse one GQL program and wrap failures with source text for miette rendering.
///
/// # Errors
///
/// Returns [`DiagnosticReport`] when parsing, AST construction, or Flagger
/// validation fails.
pub fn parse_with_source(
    source: Arc<str>,
    label: impl Into<String>,
) -> Result<Statement, DiagnosticReport> {
    parse(&source).map_err(|error| DiagnosticReport::new(error, source, label))
}

pub use many::parse_many;

fn pest_error(source: &str, error: pest::error::Error<Rule>) -> ParserError {
    let span = match error.location {
        InputLocation::Pos(offset) => point_span(offset),
        InputLocation::Span((start, end)) => SourceSpan::new(to_u32(start), to_u32(end - start)),
    };
    let message = if source.is_empty() {
        "empty GQL program".to_owned()
    } else {
        error.variant.message().to_string()
    };
    ParserError::syntax(
        message,
        span,
        Some("check GQL syntax near the highlighted span".into()),
    )
}

fn point_span(offset: usize) -> SourceSpan {
    SourceSpan::new(to_u32(offset), 0)
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
