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

/// Collect ordered node and edge contributors over a complete pattern tree.
///
/// Shared by per-path-pattern path-mode lowering ([`wrap_in_path_mode_filter`])
/// and pattern-wide match-mode lowering (`match_mode::wrap_in_match_mode_filter`).
/// Recursing through `HashJoin`/`Outer` left+right yields the union of every
/// path pattern's contributors, which is the pattern-wide edge set ISO §16.4 GR4
/// requires for DIFFERENT EDGES.
pub(super) fn collect_path_contributors(
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
        JoinTree::Questioned {
            child,
            edge,
            final_binding,
            ..
        } => {
            collect_path_contributors(child, span, contributors)?;
            contributors.push(questioned_contributor(edge, *final_binding, span)?);
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
        JoinTree::PathModeFilter { child, .. }
        | JoinTree::PathSearch { child, .. }
        | JoinTree::MatchModeFilter { child, .. } => {
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
        JoinTree::DisjunctiveScan { .. } => {
            // DisjunctiveScan is emitted by the disjunctive_label_expansion
            // optimizer rule (post-lowering); path-mode lowering runs during
            // MATCH-clause lowering, before any optimizer rule fires.
            unreachable!("DisjunctiveScan is rule-emitted post-lowering")
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

fn questioned_contributor(
    edge: &EdgeMatch,
    final_binding: TailBinding,
    span: SourceSpan,
) -> Result<PathContributor, PlannerError> {
    edge.binding
        .map(|binding| PathContributor::QuestionedEdgeNamed {
            binding,
            final_binding,
        })
        .or_else(|| {
            edge.hidden_binding
                .map(|hidden| PathContributor::QuestionedEdgeHidden {
                    hidden,
                    final_binding,
                })
        })
        .ok_or(PlannerError::NotImplemented {
            feature: "path mode over questioned edge without edge identity slot",
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
