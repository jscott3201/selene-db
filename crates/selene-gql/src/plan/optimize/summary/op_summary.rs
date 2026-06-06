//! Per-operation EXPLAIN payload summaries.
//!
//! Pulled out of `summary.rs` so the snapshot renderer stays under the file-size
//! cap. Each helper renders one pipeline-op payload string used by
//! `pipeline_summary`.

use crate::{
    LabelExpr,
    plan::{MutationOp, SessionOp, TxOp},
};

pub(super) fn mutation_summary(mutation: &MutationOp) -> String {
    match mutation {
        MutationOp::InsertNode {
            label_expr,
            property_inits,
            ..
        } => format!(
            "op=InsertNode(label={}, props={})",
            label_expr_summary(label_expr.as_ref()),
            property_inits.len()
        ),
        MutationOp::InsertEdge {
            label_expr,
            property_inits,
            ..
        } => format!(
            "op=InsertEdge(label={}, props={})",
            label_expr_summary(label_expr.as_ref()),
            property_inits.len()
        ),
        MutationOp::SetProperty { key, .. } => format!("op=SetProperty(key={})", key.as_str()),
        MutationOp::SetLabel { label, .. } => format!("op=SetLabel(label={})", label.as_str()),
        MutationOp::RemoveProperty { key, .. } => {
            format!("op=RemoveProperty(key={})", key.as_str())
        }
        MutationOp::RemoveLabel { label, .. } => {
            format!("op=RemoveLabel(label={})", label.as_str())
        }
        MutationOp::DeleteTargets { mode, targets, .. } => {
            format!("op=DeleteTargets(mode={mode:?}, targets={})", targets.len())
        }
    }
}

pub(super) fn tx_summary(tx: &TxOp) -> String {
    match tx {
        TxOp::Start { .. } => "op=Start".to_owned(),
        TxOp::Commit { .. } => "op=Commit".to_owned(),
        TxOp::Rollback { .. } => "op=Rollback".to_owned(),
    }
}

pub(super) fn session_summary(session: &SessionOp) -> String {
    match session {
        SessionOp::SetValue {
            param,
            if_not_exists,
            ..
        } => format!(
            "op=SetValue(param={}, if_not_exists={if_not_exists})",
            param.as_str()
        ),
        SessionOp::SetTimeZone { zone, .. } => format!("op=SetTimeZone(zone={zone})"),
        SessionOp::ResetAllCharacteristics { .. } => "op=ResetAllCharacteristics".to_owned(),
        SessionOp::ResetParameters { .. } => "op=ResetParameters".to_owned(),
        SessionOp::ResetTimeZone { .. } => "op=ResetTimeZone".to_owned(),
        SessionOp::ResetParameter { param, .. } => {
            format!("op=ResetParameter(param={})", param.as_str())
        }
        SessionOp::Close { .. } => "op=Close".to_owned(),
    }
}

pub(super) fn label_expr_summary(label: Option<&LabelExpr>) -> String {
    match label {
        Some(LabelExpr::Single(label)) => label.as_str().to_owned(),
        Some(LabelExpr::Conjunction(_)) => "conjunction".to_owned(),
        Some(LabelExpr::Disjunction(_)) => "disjunction".to_owned(),
        Some(LabelExpr::Negation(_)) => "negation".to_owned(),
        Some(LabelExpr::Wildcard) => "*".to_owned(),
        None => "none".to_owned(),
    }
}
