//! Source-text rendering for `CAST(<value> AS <target_type>)` expressions.
//!
//! Extracted as a sibling submodule of `ast::format` per BRIEF-135a F3 fold
//! to keep the parent `format.rs` under the 700-LOC per-file cap.

use std::fmt::{self, Write as _};

use crate::ast::{GqlType, ValueExpr};

use super::{fmt_expr, type_name};

/// Render an explicit cast expression as `CAST(<value> AS <type>)` into `out`.
pub(super) fn fmt_cast(out: &mut String, value: &ValueExpr, target_type: &GqlType) -> fmt::Result {
    out.push_str("CAST(");
    fmt_expr(out, value)?;
    out.push_str(" AS ");
    write!(out, "{}", type_name::fmt_type(target_type))?;
    out.push(')');
    Ok(())
}
