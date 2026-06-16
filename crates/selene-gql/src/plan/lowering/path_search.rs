//! Path-search selector lowering helpers.

use crate::{
    HopContributor, JoinTree, PathSelector, SourceSpan, TailBinding,
    plan::{EdgeMatch, PlannerError, RepeatEdgeMatch},
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
        JoinTree::Unit => None,
        JoinTree::Scan(scan) => scan
            .binding
            .map(TailBinding::Named)
            .or_else(|| scan.hidden_binding.map(TailBinding::Hidden)),
        JoinTree::Expand { edge, .. } => edge
            .right_binding
            .map(TailBinding::Named)
            .or_else(|| edge.right_hidden_binding.map(TailBinding::Hidden)),
        JoinTree::Questioned { final_binding, .. } => Some(*final_binding),
        JoinTree::Repeat { edge, .. } => edge
            .final_binding
            .map(TailBinding::Named)
            .or_else(|| edge.final_hidden_binding.map(TailBinding::Hidden)),
        JoinTree::PathSearch { final_binding, .. } => Some(*final_binding),
        JoinTree::PathModeFilter { child, .. } | JoinTree::MatchModeFilter { child, .. } => {
            final_binding(child, span).ok()
        }
        JoinTree::HashJoin { right, .. } | JoinTree::Outer { right, .. } => {
            final_binding(right, span).ok()
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => intersection
            .first()
            .and_then(|tree| final_binding(tree, span).ok()),
        JoinTree::Subplan(_) => None,
        JoinTree::DisjunctiveScan { .. } => {
            // DisjunctiveScan is emitted by the disjunctive_label_expansion
            // optimizer rule (post-lowering); path-search lowering runs
            // during MATCH-clause lowering, before any optimizer rule fires.
            unreachable!("DisjunctiveScan is rule-emitted post-lowering")
        }
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
        JoinTree::Unit => {}
        JoinTree::Scan(_) => {}
        JoinTree::Expand { child, edge, .. } => {
            collect_hop_contributors(child, span, contributors)?;
            contributors.push(edge_contributor(edge, span)?);
        }
        JoinTree::Questioned { child, edge, .. } => {
            collect_hop_contributors(child, span, contributors)?;
            contributors.push(questioned_contributor(edge, span)?);
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
        JoinTree::PathModeFilter { child, .. } | JoinTree::MatchModeFilter { child, .. } => {
            collect_hop_contributors(child, span, contributors)?;
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
        JoinTree::DisjunctiveScan { .. } => {
            // DisjunctiveScan is emitted by the disjunctive_label_expansion
            // optimizer rule (post-lowering); hop-contributor collection runs
            // during MATCH-clause lowering, before any optimizer rule fires.
            unreachable!("DisjunctiveScan is rule-emitted post-lowering")
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
    if let Some(binding) = edge.group_binding {
        return Ok(HopContributor::GroupNamed(binding));
    }
    if let Some(hidden) = edge.group_hidden_binding {
        return Ok(HopContributor::GroupHidden(hidden));
    }
    if max == Some(min) {
        return Ok(HopContributor::Fixed(min));
    }
    Err(PlannerError::NotImplemented {
        feature: "path selector over quantified edge without hop-count group slot",
        span,
    })
}

fn edge_contributor(edge: &EdgeMatch, span: SourceSpan) -> Result<HopContributor, PlannerError> {
    edge.binding
        .map(HopContributor::EdgeNamed)
        .or_else(|| edge.hidden_binding.map(HopContributor::EdgeHidden))
        .ok_or(PlannerError::NotImplemented {
            feature: "path selector over fixed edge without edge identity slot",
            span,
        })
}

fn questioned_contributor(
    edge: &EdgeMatch,
    span: SourceSpan,
) -> Result<HopContributor, PlannerError> {
    edge.binding
        .map(HopContributor::QuestionedNamed)
        .or_else(|| edge.hidden_binding.map(HopContributor::QuestionedHidden))
        .ok_or(PlannerError::NotImplemented {
            feature: "path selector over questioned edge without edge identity slot",
            span,
        })
}
