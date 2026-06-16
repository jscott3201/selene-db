//! Edge-index candidate helpers shared by expand executors.

use roaring::RoaringBitmap;
use selene_core::EdgeId;

use crate::{EdgeMatch, NodeOrEdgeScan, ScanAccess, ScanKind};

use super::{EvalCtx, ExecutorError, scan};

pub(super) fn candidate_row_filter(
    edge: &EdgeMatch,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Option<RoaringBitmap>, ExecutorError> {
    match &edge.access {
        ScanAccess::Linear | ScanAccess::LabelIndex { .. } => Ok(None),
        ScanAccess::TypedIndexRange { .. }
        | ScanAccess::BitmapUnion { .. }
        | ScanAccess::CompositeLookup { .. } => {
            let scan = NodeOrEdgeScan {
                binding: edge.binding,
                hidden_binding: edge.hidden_binding,
                kind: ScanKind::Edge,
                label_predicate: edge.label_predicate.clone(),
                property_predicates: edge.property_predicates.clone(),
                access: edge.access.clone(),
                span: edge.span,
            };
            Ok(Some(
                scan::candidate_rows(&scan, ctx)?.into_iter().collect(),
            ))
        }
    }
}

pub(super) fn row_filter_matches(
    filter: Option<&RoaringBitmap>,
    edge_id: EdgeId,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> bool {
    let Some(rows) = filter else {
        return true;
    };
    ctx.tx
        .snapshot()
        .row_for_edge_id(edge_id)
        .is_some_and(|row| rows.contains(row.get()))
}
