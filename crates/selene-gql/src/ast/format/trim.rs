//! Explicit TRIM formatter.

use std::fmt;

use crate::ast::{TrimSpec, ValueExpr};

pub(super) fn fmt_trim_expr(
    out: &mut String,
    spec: TrimSpec,
    character: Option<&ValueExpr>,
    source: &ValueExpr,
) -> fmt::Result {
    out.push_str("TRIM(");
    if !matches!(spec, TrimSpec::Both) || character.is_some() {
        out.push_str(match spec {
            TrimSpec::Leading => "LEADING ",
            TrimSpec::Trailing => "TRAILING ",
            TrimSpec::Both => "BOTH ",
        });
    }
    if let Some(character) = character {
        super::fmt_expr(out, character)?;
        out.push(' ');
    }
    out.push_str("FROM ");
    super::fmt_expr(out, source)?;
    out.push(')');
    Ok(())
}
