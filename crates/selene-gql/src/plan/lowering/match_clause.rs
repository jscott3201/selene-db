//! MATCH-clause lowering.

use std::collections::BTreeSet;

use selene_core::IStr;

use crate::{
    EdgePattern, GraphPattern, MatchClause, NodePattern, PathMode, PatternElement,
    analyze::{AnalyzedStatement, BindingDecl, BindingDeclKind, BindingId, BindingUseKind},
    plan::{
        BindingDef, BindingElement, EdgeMatch, FilterPredicate, JoinTree, NodeOrEdgeScan, PathPlan,
        PatternPlan, PlannerError, ScanKind,
    },
};

use super::expr;

/// Lower leading MATCH clauses into one pattern plan.
pub(crate) fn lower_match_prefix(
    clauses: &[&MatchClause],
    analyzed: &AnalyzedStatement,
) -> Result<Option<PatternPlan>, PlannerError> {
    if clauses.is_empty() {
        return Ok(None);
    }

    let mut filters = Vec::new();
    let mut paths = Vec::new();
    let mut binding_ids = BTreeSet::new();
    let mut current: Option<(JoinTree, BTreeSet<IStr>)> = None;

    for clause in clauses {
        reject_unsupported_clause(clause)?;
        let (tree, names) =
            lower_match_clause(clause, analyzed, &mut filters, &mut paths, &mut binding_ids)?;
        current = Some(match (current, clause.optional) {
            (None, _) => (tree, names),
            (Some((left, left_names)), false) => {
                let key = shared_names(&left_names, &names);
                let mut all_names = left_names;
                all_names.extend(names);
                (
                    JoinTree::HashJoin {
                        left: Box::new(left),
                        right: Box::new(tree),
                        key,
                    },
                    all_names,
                )
            }
            (Some((left, left_names)), true) => {
                let key = shared_names(&left_names, &names);
                let mut all_names = left_names;
                all_names.extend(names);
                (
                    JoinTree::Outer {
                        left: Box::new(left),
                        right: Box::new(tree),
                        key,
                    },
                    all_names,
                )
            }
        });
    }

    let Some((join_tree, _)) = current else {
        return Ok(None);
    };
    Ok(Some(PatternPlan {
        bindings: binding_defs(analyzed, &binding_ids),
        join_tree,
        filters,
        paths,
    }))
}

fn lower_match_clause(
    clause: &MatchClause,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    paths: &mut Vec<PathPlan>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<(JoinTree, BTreeSet<IStr>), PlannerError> {
    let mut current: Option<(JoinTree, BTreeSet<IStr>)> = None;
    for pattern in &clause.patterns {
        let (tree, names) = lower_graph_pattern(pattern, analyzed, filters, paths, binding_ids)?;
        current = Some(match current {
            None => (tree, names),
            Some((left, left_names)) => {
                let key = shared_names(&left_names, &names);
                let mut all_names = left_names;
                all_names.extend(names);
                (
                    JoinTree::HashJoin {
                        left: Box::new(left),
                        right: Box::new(tree),
                        key,
                    },
                    all_names,
                )
            }
        });
    }
    if let Some(where_clause) = &clause.where_clause {
        filters.push(expr::filter_predicate(where_clause, analyzed)?);
    }
    current.ok_or(PlannerError::NotImplemented {
        feature: "empty graph pattern",
        span: clause.span,
    })
}

fn lower_graph_pattern(
    pattern: &GraphPattern,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    paths: &mut Vec<PathPlan>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<(JoinTree, BTreeSet<IStr>), PlannerError> {
    if let Some(name) = pattern.path_binding {
        let binding = binding_for_decl(name, pattern.span, BindingDeclKind::PathBinding, analyzed)?;
        binding_ids.insert(binding);
        paths.push(PathPlan {
            binding,
            span: pattern.span,
        });
    }

    let mut elements = pattern.elements.iter();
    let Some(PatternElement::Node(first)) = elements.next() else {
        return Err(PlannerError::NotImplemented {
            feature: "empty graph pattern",
            span: pattern.span,
        });
    };
    let mut names = BTreeSet::new();
    let mut current = JoinTree::Scan(node_scan(
        first,
        analyzed,
        filters,
        &mut names,
        binding_ids,
    )?);
    while let Some(element) = elements.next() {
        let PatternElement::Edge(edge) = element else {
            return Err(PlannerError::NotImplemented {
                feature: "non-alternating graph pattern",
                span: pattern.span,
            });
        };
        if edge.quantifier.is_some() {
            return Err(PlannerError::NotImplemented {
                feature: "variable-length edge patterns (quantifier)",
                span: edge.span,
            });
        }
        let Some(PatternElement::Node(right)) = elements.next() else {
            return Err(PlannerError::NotImplemented {
                feature: "edge without target",
                span: edge.span,
            });
        };
        let left_binding = chain_tail_binding(&current);
        let right_binding = node_binding(right, analyzed, &mut names, binding_ids)?;
        let edge_match = edge_match(
            edge,
            left_binding,
            right_binding,
            analyzed,
            filters,
            &mut names,
            binding_ids,
        )?;
        current = JoinTree::Expand {
            child: Box::new(current),
            direction: edge.direction,
            edge: edge_match,
        };
        collect_node_predicates(right, right_binding, analyzed, filters)?;
    }
    Ok((current, names))
}

fn node_scan(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    names: &mut BTreeSet<IStr>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<NodeOrEdgeScan, PlannerError> {
    let binding = node_binding(node, analyzed, names, binding_ids)?;
    let property_predicates = node
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(binding, *key, value, analyzed))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(where_clause) = &node.inline_where {
        filters.push(expr::filter_predicate(where_clause, analyzed)?);
    }
    Ok(NodeOrEdgeScan {
        binding,
        kind: ScanKind::Node,
        label_predicate: node.label_expr.clone(),
        property_predicates,
        span: node.span,
    })
}

fn collect_node_predicates(
    node: &NodePattern,
    binding: Option<BindingId>,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
) -> Result<(), PlannerError> {
    for (key, value) in &node.properties {
        filters.push(expr::property_predicate(binding, *key, value, analyzed)?);
    }
    if let Some(where_clause) = &node.inline_where {
        filters.push(expr::filter_predicate(where_clause, analyzed)?);
    }
    Ok(())
}

fn edge_match(
    edge: &EdgePattern,
    left_binding: Option<BindingId>,
    right_binding: Option<BindingId>,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    names: &mut BTreeSet<IStr>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<EdgeMatch, PlannerError> {
    let binding = edge_binding(edge, analyzed, names, binding_ids)?;
    let property_predicates = edge
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(binding, *key, value, analyzed))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(where_clause) = &edge.inline_where {
        filters.push(expr::filter_predicate(where_clause, analyzed)?);
    }
    Ok(EdgeMatch {
        binding,
        label_predicate: edge.label_expr.clone(),
        property_predicates,
        left_binding,
        right_binding,
        span: edge.span,
    })
}

fn node_binding(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
    names: &mut BTreeSet<IStr>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<Option<BindingId>, PlannerError> {
    node.binding
        .map(|name| {
            names.insert(name);
            let binding =
                binding_for_pattern(name, node.span, BindingDeclKind::NodePattern, analyzed)?;
            binding_ids.insert(binding);
            Ok(binding)
        })
        .transpose()
}

fn edge_binding(
    edge: &EdgePattern,
    analyzed: &AnalyzedStatement,
    names: &mut BTreeSet<IStr>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<Option<BindingId>, PlannerError> {
    edge.binding
        .map(|name| {
            names.insert(name);
            let binding =
                binding_for_pattern(name, edge.span, BindingDeclKind::EdgePattern, analyzed)?;
            binding_ids.insert(binding);
            Ok(binding)
        })
        .transpose()
}

fn binding_for_pattern(
    name: IStr,
    span: crate::SourceSpan,
    expected: BindingDeclKind,
    analyzed: &AnalyzedStatement,
) -> Result<BindingId, PlannerError> {
    if let Some(binding) = analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| {
            decl.name() == name && decl.span() == span && same_element(decl.kind(), expected)
        })
        .map(BindingDecl::id)
    {
        return Ok(binding);
    }
    analyzed
        .references
        .iter()
        .find(|reference| {
            reference.name == name
                && reference.span == span
                && reference.kind == BindingUseKind::PatternReuse
        })
        .map(|reference| reference.binding)
        .ok_or(PlannerError::BindingResolutionLost {
            binding: BindingId::new(u32::MAX),
            span,
        })
}

fn binding_for_decl(
    name: IStr,
    span: crate::SourceSpan,
    expected: BindingDeclKind,
    analyzed: &AnalyzedStatement,
) -> Result<BindingId, PlannerError> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.name() == name && decl.span() == span && decl.kind() == expected)
        .map(BindingDecl::id)
        .ok_or(PlannerError::BindingResolutionLost {
            binding: BindingId::new(u32::MAX),
            span,
        })
}

fn same_element(found: BindingDeclKind, expected: BindingDeclKind) -> bool {
    matches!(
        (found, expected),
        (
            BindingDeclKind::NodePattern | BindingDeclKind::InsertNode,
            BindingDeclKind::NodePattern
        ) | (
            BindingDeclKind::EdgePattern | BindingDeclKind::InsertEdge,
            BindingDeclKind::EdgePattern
        ) | (BindingDeclKind::PathBinding, BindingDeclKind::PathBinding)
    )
}

fn binding_defs(
    analyzed: &AnalyzedStatement,
    binding_ids: &BTreeSet<BindingId>,
) -> Vec<BindingDef> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .filter(|decl| binding_ids.contains(&decl.id()))
        .filter_map(|decl| {
            let element = match decl.kind() {
                BindingDeclKind::NodePattern | BindingDeclKind::InsertNode => BindingElement::Node,
                BindingDeclKind::EdgePattern | BindingDeclKind::InsertEdge => BindingElement::Edge,
                BindingDeclKind::PathBinding => BindingElement::Path,
                BindingDeclKind::LetAlias
                | BindingDeclKind::UnwindAlias
                | BindingDeclKind::ProjectionAlias
                | BindingDeclKind::YieldColumn => return None,
            };
            Some(BindingDef {
                binding: decl.id(),
                name: decl.name(),
                element,
                ty: decl.ty().clone(),
                label_predicate: decl.label_expr().cloned(),
                span: decl.span(),
            })
        })
        .collect()
}

fn reject_unsupported_clause(clause: &MatchClause) -> Result<(), PlannerError> {
    if clause.selector.is_some() {
        return Err(PlannerError::NotImplemented {
            feature: "MATCH path selector (SHORTEST/ALL/ANY)",
            span: clause.span,
        });
    }
    if clause.match_mode.is_some() {
        return Err(PlannerError::NotImplemented {
            feature: "MATCH mode (REPEATABLE ELEMENTS / DIFFERENT EDGES)",
            span: clause.span,
        });
    }
    if clause.path_mode != PathMode::Walk {
        return Err(PlannerError::NotImplemented {
            feature: "MATCH path mode (TRAIL/SIMPLE/ACYCLIC)",
            span: clause.span,
        });
    }
    Ok(())
}

fn shared_names(left: &BTreeSet<IStr>, right: &BTreeSet<IStr>) -> Vec<IStr> {
    left.intersection(right).copied().collect()
}

/// Return the binding of the most-recently expanded chain tail, propagating
/// `None` when the trailing element is anonymous so the caller does not
/// silently fall back to an older named node from earlier in the chain.
fn chain_tail_binding(tree: &JoinTree) -> Option<BindingId> {
    match tree {
        JoinTree::Scan(scan) => scan.binding,
        JoinTree::Expand { edge, .. } => edge.right_binding,
        JoinTree::HashJoin { right, .. } | JoinTree::Outer { right, .. } => {
            chain_tail_binding(right)
        }
        JoinTree::WorstCaseOptimal { intersection } => {
            intersection.first().and_then(chain_tail_binding)
        }
        JoinTree::Subplan(_) => None,
    }
}
