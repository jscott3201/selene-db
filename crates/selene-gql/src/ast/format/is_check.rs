//! `IS ...` predicate formatting.

use std::fmt::{self, Write as _};

use crate::{IsCheckKind, NormalForm, TruthValue, ValueExpr};

use super::{fmt_expr, fmt_label_expr, fmt_type};

pub(super) fn fmt_is_check(
    out: &mut String,
    operand: &ValueExpr,
    kind: &IsCheckKind,
    negated: bool,
) -> fmt::Result {
    fmt_expr(out, operand)?;
    out.push_str(" IS");
    if negated {
        out.push_str(" NOT");
    }
    match kind {
        IsCheckKind::Null => out.push_str(" NULL"),
        IsCheckKind::Directed => out.push_str(" DIRECTED"),
        IsCheckKind::Labeled(label) => {
            out.push_str(" LABELED ");
            fmt_label_expr(out, label)?;
        }
        IsCheckKind::TruthValue(value) => {
            out.push(' ');
            out.push_str(match value {
                TruthValue::True => "TRUE",
                TruthValue::False => "FALSE",
                TruthValue::Unknown => "UNKNOWN",
            });
        }
        IsCheckKind::Typed(ty) => write!(out, " TYPED {}", fmt_type(ty))?,
        IsCheckKind::Normalized(form) => {
            out.push(' ');
            out.push_str(fmt_normal_form(*form));
            out.push_str(" NORMALIZED");
        }
        IsCheckKind::SourceOf(value) => {
            out.push_str(" SOURCE OF ");
            fmt_expr(out, value)?;
        }
        IsCheckKind::DestinationOf(value) => {
            out.push_str(" DESTINATION OF ");
            fmt_expr(out, value)?;
        }
    }
    Ok(())
}

pub(super) fn fmt_normal_form(form: NormalForm) -> &'static str {
    match form {
        NormalForm::Nfc => "NFC",
        NormalForm::Nfd => "NFD",
        NormalForm::Nfkc => "NFKC",
        NormalForm::Nfkd => "NFKD",
    }
}
