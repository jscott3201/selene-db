//! Session-control statement executor (ISO/IEC 39075:2024 section 7).
//!
//! Each session command mutates [`Session`] state directly and returns
//! [`StatementOutput::Empty`]. `SESSION SET VALUE` evaluates its right-hand
//! side against an empty binding row (the value is restricted to a
//! `<value specification>`, so GS14 is not claimed) and binds the result as a
//! session-local parameter. `SESSION SET TIME ZONE` parses the time-zone
//! string with `jiff`. The RESET forms clear parameters / reset the time zone,
//! and `SESSION CLOSE` raises the termination flag.

use selene_core::Value;

use crate::{
    ImplDefinedCaps, ProcedureRegistry, SessionOp, SourceSpan, ValueExpr,
    plan::SubqueryRegistry,
    runtime::{
        Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError, Session,
        StatementOutput, TxContext, evaluator,
    },
};

/// Execute one top-level session-control operation.
pub(crate) fn execute(
    op: &SessionOp,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    match op {
        SessionOp::SetValue {
            param,
            value,
            if_not_exists,
            span,
        } => set_value(
            session,
            registry,
            param.clone(),
            value,
            *if_not_exists,
            *span,
        ),
        SessionOp::SetTimeZone { zone, span } => set_time_zone(session, zone, *span),
        SessionOp::ResetAllCharacteristics { .. } => {
            session.reset_characteristics();
            Ok(StatementOutput::Empty)
        }
        SessionOp::ResetParameters { .. } => {
            session.reset_parameters();
            Ok(StatementOutput::Empty)
        }
        SessionOp::ResetTimeZone { .. } => {
            session.reset_time_zone();
            Ok(StatementOutput::Empty)
        }
        SessionOp::ResetParameter { param, .. } => {
            session.reset_parameter(param);
            Ok(StatementOutput::Empty)
        }
        SessionOp::Close { .. } => {
            session.close();
            Ok(StatementOutput::Empty)
        }
    }
}

fn set_value(
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
    param: selene_core::IStr,
    value: &ValueExpr,
    if_not_exists: bool,
    _span: SourceSpan,
) -> Result<StatementOutput, ExecutorError> {
    // IF NOT EXISTS (ISO section 7.4): leave an existing binding untouched.
    if if_not_exists && session.has_parameter(&param) {
        return Ok(StatementOutput::Empty);
    }
    let evaluated = evaluate_constant(session, registry, value)?;
    session.bind_parameter(param, evaluated);
    Ok(StatementOutput::Empty)
}

/// Evaluate a `<value specification>` right-hand side against an empty binding.
///
/// The RHS may reference already-bound session parameters (`$other`) and
/// literals; it carries no row context (no graph variables) because session
/// commands run outside a binding-producing pipeline.
fn evaluate_constant(
    session: &Session<'_>,
    registry: &dyn ProcedureRegistry,
    value: &ValueExpr,
) -> Result<Value, ExecutorError> {
    let snapshot = session.graph().read();
    let providers = session.graph().index_providers();
    let session_tz = session.effective_time_zone();
    let caps = ImplDefinedCaps::default();
    let ctx = TxContext::read_only_with_parameters(
        snapshot,
        &caps,
        registry,
        providers,
        &session.scalar_parameters,
    )
    .with_session_time_zone(session_tz);
    let expr_ids = crate::analyze::ExprIdLookup::default();
    let subqueries = SubqueryRegistry::default();
    let eval_ctx = EvalCtx {
        tx: &ctx,
        expr_ids: &expr_ids,
        subqueries: &subqueries,
    };
    let binding = Binding::empty();
    let schema = BindingTableSchema {
        columns: Vec::new(),
    };
    evaluator::evaluate(value, &binding, &schema, &eval_ctx)
}

fn set_time_zone(
    session: &mut Session<'_>,
    zone: &str,
    span: SourceSpan,
) -> Result<StatementOutput, ExecutorError> {
    let tz = parse_time_zone(zone, span)?;
    session.set_time_zone(tz);
    Ok(StatementOutput::Empty)
}

/// Parse a `<time zone string>` into a `jiff` time zone.
///
/// Accepts, in priority order: a fixed UTC offset (`+HH:MM`, `-HH:MM:SS`,
/// `+HHMM`, or `Z`), an IANA region name (e.g. `America/New_York`), and a POSIX
/// TZ string. An unparseable string raises a data exception rather than
/// panicking (forbid-unsafe / no-panic discipline).
fn parse_time_zone(zone: &str, span: SourceSpan) -> Result<jiff::tz::TimeZone, ExecutorError> {
    if let Some(offset) = parse_fixed_offset(zone) {
        return Ok(jiff::tz::TimeZone::fixed(offset));
    }
    jiff::tz::TimeZone::get(zone)
        .or_else(|_| jiff::tz::TimeZone::posix(zone))
        .map_err(|_| {
            ExecutorError::data_exception(
                DataExceptionSubclass::InvalidTimeZone,
                format!("unknown or invalid time zone: {zone}"),
                span,
            )
        })
}

/// Parse a fixed UTC offset string into a `jiff` offset.
///
/// Supports `Z`/`z` (UTC) and `(+|-)HH[:]MM[[:]SS]`. Returns `None` for any
/// other form so the caller can fall back to IANA / POSIX resolution.
fn parse_fixed_offset(zone: &str) -> Option<jiff::tz::Offset> {
    if zone.eq_ignore_ascii_case("Z") {
        return Some(jiff::tz::Offset::UTC);
    }
    let (sign, rest) = match zone.as_bytes().first()? {
        b'+' => (1_i32, &zone[1..]),
        b'-' => (-1_i32, &zone[1..]),
        _ => return None,
    };
    // Accept `HH:MM`, `HH:MM:SS`, `HHMM`, or `HHMMSS`.
    let digits: String = rest.chars().filter(|ch| *ch != ':').collect();
    if digits.len() < 2 || digits.len() > 6 || !digits.len().is_multiple_of(2) {
        return None;
    }
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[0..2].parse().ok()?;
    let minutes: i32 = digits.get(2..4).map_or(Ok(0), str::parse).ok()?;
    let seconds: i32 = digits.get(4..6).map_or(Ok(0), str::parse).ok()?;
    if hours > 25 || minutes > 59 || seconds > 59 {
        return None;
    }
    let total = sign * (hours * 3600 + minutes * 60 + seconds);
    jiff::tz::Offset::from_seconds(total).ok()
}
