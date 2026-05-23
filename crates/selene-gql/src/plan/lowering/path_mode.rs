//! Path-mode lowering helpers.

use crate::{
    EdgeDirection, PathMode, SourceSpan,
    plan::{EdgeMatch, JoinTree, PathContributor, PlannerError, RepeatEdgeMatch, TailBinding},
};

pub(super) fn wrap_in_path_mode_filter(
    child: JoinTree,
    path_mode: PathMode,
    span: SourceSpan,
) -> Result<JoinTree, PlannerError> {
    if path_mode == PathMode::Walk {
        return Ok(child);
    }
    let mut path_contributors = Vec::new();
    collect_path_contributors(&child, span, &mut path_contributors)?;
    Ok(JoinTree::PathModeFilter {
        path_mode,
        child: Box::new(child),
        path_contributors,
    })
}

fn collect_path_contributors(
    tree: &JoinTree,
    span: SourceSpan,
    contributors: &mut Vec<PathContributor>,
) -> Result<(), PlannerError> {
    match tree {
        JoinTree::Scan(scan) => contributors.push(node_contributor(
            scan.binding,
            scan.hidden_binding,
            "path mode over bindingless source node",
            span,
        )?),
        JoinTree::Expand { child, edge, .. } => {
            collect_path_contributors(child, span, contributors)?;
            contributors.push(edge_contributor(edge, span)?);
            contributors.push(node_contributor(
                edge.right_binding,
                edge.right_hidden_binding,
                "path mode over bindingless target node",
                span,
            )?);
        }
        JoinTree::Repeat {
            child,
            edge,
            direction,
            ..
        } => {
            collect_path_contributors(child, span, contributors)?;
            contributors.push(repeat_contributor(edge, *direction, span)?);
        }
        JoinTree::PathModeFilter { child, .. } | JoinTree::PathSearch { child, .. } => {
            collect_path_contributors(child, span, contributors)?;
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            collect_path_contributors(left, span, contributors)?;
            collect_path_contributors(right, span, contributors)?;
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for branch in intersection {
                collect_path_contributors(branch, span, contributors)?;
            }
        }
        JoinTree::Subplan(_) => {
            return Err(PlannerError::NotImplemented {
                feature: "path mode over non-path subquery; ISO path modes apply to path patterns",
                span,
            });
        }
    }
    Ok(())
}

fn node_contributor(
    binding: Option<crate::BindingId>,
    hidden: Option<crate::HiddenBindingId>,
    feature: &'static str,
    span: SourceSpan,
) -> Result<PathContributor, PlannerError> {
    binding
        .map(TailBinding::Named)
        .or_else(|| hidden.map(TailBinding::Hidden))
        .map(PathContributor::Node)
        .ok_or(PlannerError::NotImplemented { feature, span })
}

fn edge_contributor(edge: &EdgeMatch, span: SourceSpan) -> Result<PathContributor, PlannerError> {
    edge.binding
        .map(PathContributor::EdgeNamed)
        .or_else(|| edge.hidden_binding.map(PathContributor::EdgeHidden))
        .ok_or(PlannerError::NotImplemented {
            feature: "path mode over fixed edge without edge identity slot",
            span,
        })
}

fn repeat_contributor(
    edge: &RepeatEdgeMatch,
    direction: EdgeDirection,
    span: SourceSpan,
) -> Result<PathContributor, PlannerError> {
    let source = edge
        .left_binding
        .map(TailBinding::Named)
        .or_else(|| edge.left_hidden_binding.map(TailBinding::Hidden))
        .ok_or(PlannerError::NotImplemented {
            feature: "path mode over quantified edge without source node binding",
            span,
        })?;
    if let Some(binding) = edge.group_binding {
        return Ok(PathContributor::EdgeGroupNamed {
            binding,
            source,
            direction,
        });
    }
    if let Some(hidden) = edge.group_hidden_binding {
        return Ok(PathContributor::EdgeGroupHidden {
            hidden,
            source,
            direction,
        });
    }
    Err(PlannerError::NotImplemented {
        feature: "path mode over quantified edge without edge group slot",
        span,
    })
}
