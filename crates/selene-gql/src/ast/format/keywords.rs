//! Keyword rendering helpers for the read-side formatter.

use std::fmt::{self, Write};

use crate::{BinaryOp, MatchMode, PathMode, PathSelector, SetOp};

pub(super) fn fmt_set_op(op: SetOp) -> &'static str {
    match op {
        SetOp::Union => "UNION",
        SetOp::UnionAll => "UNION ALL",
        SetOp::Intersect => "INTERSECT",
        SetOp::IntersectAll => "INTERSECT ALL",
        SetOp::Except => "EXCEPT",
        SetOp::ExceptAll => "EXCEPT ALL",
        SetOp::Otherwise => "OTHERWISE",
    }
}

pub(super) fn fmt_binary(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        // `Mod` and `Power` are runtime-only operators that back the ISO
        // `MOD(x, y)` and `POWER(x, y)` scalar functions; the grammar never
        // emits them as infix AST nodes, so they are rendered in function
        // form by the `format.rs` `BinaryOp` arm and never reach this helper.
        BinaryOp::Mod | BinaryOp::Power => "",
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "<>",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "AND",
        BinaryOp::Or => "OR",
        BinaryOp::Xor => "XOR",
        BinaryOp::Concat => "||",
        BinaryOp::Contains => "CONTAINS",
        BinaryOp::StartsWith => "STARTS WITH",
        BinaryOp::EndsWith => "ENDS WITH",
    }
}

pub(super) fn fmt_path_selector(out: &mut String, selector: PathSelector) -> fmt::Result {
    // Per ISO 39075:2024 §16.6: the counted forms carry a runtime count, so the
    // selector is rendered directly into the buffer rather than via a static
    // keyword string. `SHORTEST N` is the counted path search (G019); `SHORTEST
    // N GROUPS` is the counted group search (G020).
    match selector {
        PathSelector::Any => out.push_str("ANY"),
        PathSelector::All => out.push_str("ALL"),
        PathSelector::AnyShortest => out.push_str("ANY SHORTEST"),
        PathSelector::AllShortest => out.push_str("ALL SHORTEST"),
        PathSelector::CountedShortest { paths } => write!(out, "SHORTEST {paths}")?,
        PathSelector::CountedShortestGroup { groups } => write!(out, "SHORTEST {groups} GROUPS")?,
    }
    Ok(())
}

pub(super) fn fmt_match_mode(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::DifferentEdges => "DIFFERENT EDGES",
        MatchMode::RepeatableElements => "REPEATABLE ELEMENTS",
    }
}

pub(super) fn fmt_path_mode(mode: PathMode) -> &'static str {
    match mode {
        PathMode::Walk => "WALK",
        PathMode::Trail => "TRAIL",
        PathMode::Acyclic => "ACYCLIC",
        PathMode::Simple => "SIMPLE",
    }
}
