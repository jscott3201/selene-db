//! Path-search selector lowering helpers.

use crate::{
    HopContributor, JoinTree, PathSelector, SourceSpan, TailBinding,
    plan::{PlannerError, RepeatEdgeMatch},
};

pub(super) fn wrap_in_path_search(
    child: JoinTree,
    selector: PathSelector,
    source_binding: TailBinding,
    span: SourceSpan,
) -> Result<JoinTree, PlannerError> {
    let final_binding = final_binding(&child, span)?;
    let hop_contributors = compute_hop_contributors(&child, span)?;
    Ok(JoinTree::PathSearch {
        selector,
        child: Box::new(child),
        source_binding,
        final_binding,
        hop_contributors,
    })
}

fn final_binding(tree: &JoinTree, span: SourceSpan) -> Result<TailBinding, PlannerError> {
    match tree {
        JoinTree::Scan(scan) => scan
            .binding
            .map(TailBinding::Named)
            .or_else(|| scan.hidden_binding.map(TailBinding::Hidden)),
        JoinTree::Expand { edge, .. } => edge
            .right_binding
            .map(TailBinding::Named)
            .or_else(|| edge.right_hidden_binding.map(TailBinding::Hidden)),
        JoinTree::Repeat { edge, .. } => edge
            .final_binding
            .map(TailBinding::Named)
            .or_else(|| edge.final_hidden_binding.map(TailBinding::Hidden)),
        JoinTree::PathSearch { final_binding, .. } => Some(*final_binding),
        JoinTree::HashJoin { right, .. } | JoinTree::Outer { right, .. } => {
            final_binding(right, span).ok()
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => intersection
            .first()
            .and_then(|tree| final_binding(tree, span).ok()),
        JoinTree::Subplan(_) => None,
    }
    .ok_or(PlannerError::NotImplemented {
        feature: "path selector without source/final node binding",
        span,
    })
}

fn compute_hop_contributors(
    tree: &JoinTree,
    span: SourceSpan,
) -> Result<Vec<HopContributor>, PlannerError> {
    let mut contributors = Vec::new();
    collect_hop_contributors(tree, span, &mut contributors)?;
    Ok(contributors)
}

fn collect_hop_contributors(
    tree: &JoinTree,
    span: SourceSpan,
    contributors: &mut Vec<HopContributor>,
) -> Result<(), PlannerError> {
    match tree {
        JoinTree::Scan(_) => {}
        JoinTree::Expand { child, .. } => {
            collect_hop_contributors(child, span, contributors)?;
            contributors.push(HopContributor::Fixed(1));
        }
        JoinTree::Repeat {
            child,
            edge,
            min,
            max,
            ..
        } => {
            collect_hop_contributors(child, span, contributors)?;
            contributors.push(repeat_contributor(edge, *min, *max, span)?);
        }
        JoinTree::PathSearch {
            child,
            hop_contributors,
            ..
        } => {
            collect_hop_contributors(child, span, contributors)?;
            contributors.extend(hop_contributors.iter().cloned());
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            collect_hop_contributors(left, span, contributors)?;
            collect_hop_contributors(right, span, contributors)?;
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for branch in intersection {
                collect_hop_contributors(branch, span, contributors)?;
            }
        }
        JoinTree::Subplan(_) => {
            return Err(PlannerError::NotImplemented {
                feature: "path selector over non-path subquery; ISO path selectors apply to path patterns",
                span,
            });
        }
    }
    Ok(())
}

fn repeat_contributor(
    edge: &RepeatEdgeMatch,
    min: u32,
    max: Option<u32>,
    span: SourceSpan,
) -> Result<HopContributor, PlannerError> {
    if max == Some(min) {
        return Ok(HopContributor::Fixed(min));
    }
    edge.group_binding
        .map(HopContributor::GroupNamed)
        .or_else(|| edge.group_hidden_binding.map(HopContributor::GroupHidden))
        .ok_or(PlannerError::NotImplemented {
            feature: "path selector over quantified edge without hop-count group slot",
            span,
        })
}
