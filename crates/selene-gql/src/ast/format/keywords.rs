//! Keyword rendering helpers for the read-side formatter.

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

pub(super) fn fmt_path_selector(selector: PathSelector) -> &'static str {
    match selector {
        PathSelector::Any => "ANY",
        PathSelector::All => "ALL",
        PathSelector::AnyShortest => "ANY SHORTEST",
        PathSelector::AllShortest => "ALL SHORTEST",
    }
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
