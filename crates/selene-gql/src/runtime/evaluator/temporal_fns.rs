//! Current-datetime scalar functions (ISO/IEC 39075:2024 section 20.27).
//!
//! These functions read the per-session time-zone displacement threaded into
//! [`TxContext`](crate::runtime::TxContext) (default UTC per Annex B ID048) and
//! produce temporal [`Value`]s anchored to "now". Each call within one
//! statement re-reads the wall clock; the session time zone selects how the
//! instant is presented (zoned forms) and which local wall-clock components are
//! returned (local forms).

use selene_core::Value;

use crate::runtime::{EvalCtx, ExecutorError};

/// `current_timestamp()`: the current zoned datetime in the session time zone
/// (ISO section 20.27).
pub(super) fn eval_current_timestamp(
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    Ok(Value::ZonedDateTime(Box::new(now_zoned(ctx))))
}

/// `localtimestamp()`: the current local (zoneless) datetime, with wall-clock
/// components taken in the session time zone (ISO section 20.27).
pub(super) fn eval_localtimestamp(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::LocalDateTime(now_zoned(ctx).datetime()))
}

/// `current_date()`: the current date in the session time zone (ISO section 20.27).
pub(super) fn eval_current_date(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::Date(now_zoned(ctx).date()))
}

/// `current_time()`: the current zoned time in the session time zone
/// (ISO section 20.27).
///
/// `jiff` 0.2 has no dedicated zoned-time type, so selene-core models a zoned
/// time as a [`jiff::Zoned`] (matching `Value::ZonedTime`).
pub(super) fn eval_current_time(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::ZonedTime(Box::new(now_zoned(ctx))))
}

/// `localtime()`: the current local (zoneless) time, with wall-clock components
/// taken in the session time zone (ISO section 20.27).
pub(super) fn eval_localtime(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::LocalTime(now_zoned(ctx).time()))
}

/// Capture "now" rendered in the session time zone.
fn now_zoned(ctx: &EvalCtx<'_, '_, '_, '_>) -> jiff::Zoned {
    jiff::Timestamp::now().to_zoned(ctx.tx.session_time_zone().clone())
}
