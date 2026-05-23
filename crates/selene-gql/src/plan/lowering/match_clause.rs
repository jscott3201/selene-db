//! MATCH-clause lowering.

use std::collections::BTreeSet;

use selene_core::IStr;

use crate::{
    EdgePattern, GraphPattern, LabelExpr, MatchClause, NodePattern, PathMode, PathSelector,
    PatternElement, Quantifier,
    analyze::{AnalyzedStatement, BindingDecl, BindingDeclKind, BindingId, BindingUseKind},
    plan::{
        BindingDef, BindingElement, BuildSide, EdgeMatch, FilterPredicate, HiddenBindingId,
        JoinTree, NodeOrEdgeScan, PathPlan, PatternPlan, PlannerError, ScanAccess, ScanKind,
        TailBinding,
    },
};

struct LoweredClause {
    tree: JoinTree,
    names: BTreeSet<IStr>,
    filters: Vec<FilterPredicate>,
}

struct GraphLoweringContext<'a, 's> {
    path_mode: PathMode,
    selector: Option<PathSelector>,
    analyzed: &'a AnalyzedStatement,
    filters: &'s mut Vec<FilterPredicate>,
    paths: &'s mut Vec<PathPlan>,
    binding_ids: &'s mut BTreeSet<BindingId>,
    hidden: &'s mut HiddenAllocator,
}

/// Predicates collected from the syntactic right-side node of an edge
/// expansion. Bundled so they ride the `EdgeMatch` instead of leaking into
/// the unscoped pattern filter list.
pub(super) struct RightNode {
    pub(super) binding: Option<BindingId>,
    pub(super) hidden_binding: Option<HiddenBindingId>,
    pub(super) label_predicate: Option<LabelExpr>,
    pub(super) property_predicates: Vec<FilterPredicate>,
}

pub(super) struct EdgeLoweringContext<'a, 's> {
    pub(super) analyzed: &'a AnalyzedStatement,
    pub(super) filters: &'s mut Vec<FilterPredicate>,
    pub(super) names: &'s mut BTreeSet<IStr>,
    pub(super) binding_ids: &'s mut BTreeSet<BindingId>,
    pub(super) hidden: &'s mut HiddenAllocator,
}

#[derive(Default)]
pub(super) struct HiddenAllocator {
    next: u32,
}

impl HiddenAllocator {
    pub(super) fn next(&mut self) -> HiddenBindingId {
        let id = HiddenBindingId::new(self.next);
        self.next += 1;
        id
    }
}

use super::{expr, path_search, repeat};

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
    let mut hidden = HiddenAllocator::default();
    let mut current: Option<(JoinTree, BTreeSet<IStr>)> = None;

    for clause in clauses {
        reject_unsupported_clause(clause)?;
        let lowered =
            lower_match_clause(clause, analyzed, &mut paths, &mut binding_ids, &mut hidden)?;
        current = Some(match (current, clause.optional) {
            (None, false) => {
                filters.extend(lowered.filters);
                (lowered.tree, lowered.names)
            }
            (None, true) => {
                // Why: a leading OPTIONAL MATCH lacks a left input to outer-join
                // against. GQL semantics call for one null-extended row; the
                // planner needs a unit-row scan or special leading marker we
                // do not yet model. Defer until the executor surface lands.
                return Err(PlannerError::NotImplemented {
                    feature: "leading OPTIONAL MATCH (no preceding pipeline)",
                    span: clause.span,
                });
            }
            (Some((left, left_names)), false) => {
                let key = shared_names(&left_names, &lowered.names);
                let mut all_names = left_names;
                all_names.extend(lowered.names);
                filters.extend(lowered.filters);
                (
                    JoinTree::HashJoin {
                        left: Box::new(left),
                        right: Box::new(lowered.tree),
                        key,
                        build_side: BuildSide::Left,
                    },
                    all_names,
                )
            }
            (Some((left, left_names)), true) => {
                let key = shared_names(&left_names, &lowered.names);
                let (right_filters, global_filters) =
                    split_optional_filters(lowered.filters, &left_names, analyzed);
                filters.extend(global_filters);
                let mut all_names = left_names;
                all_names.extend(lowered.names);
                (
                    JoinTree::Outer {
                        left: Box::new(left),
                        right: Box::new(lowered.tree),
                        key,
                        right_filters,
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
    paths: &mut Vec<PathPlan>,
    binding_ids: &mut BTreeSet<BindingId>,
    hidden: &mut HiddenAllocator,
) -> Result<LoweredClause, PlannerError> {
    let mut filters = Vec::new();
    let mut current: Option<(JoinTree, BTreeSet<IStr>)> = None;
    for pattern in &clause.patterns {
        let mut ctx = GraphLoweringContext {
            path_mode: clause.path_mode,
            selector: clause.selector,
            analyzed,
            filters: &mut filters,
            paths,
            binding_ids,
            hidden,
        };
        let (tree, names) = lower_graph_pattern(pattern, &mut ctx)?;
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
                        build_side: BuildSide::Left,
                    },
                    all_names,
                )
            }
        });
    }
    if let Some(where_clause) = &clause.where_clause {
        filters.push(expr::filter_predicate(where_clause, analyzed)?);
    }
    let (tree, names) = current.ok_or(PlannerError::NotImplemented {
        feature: "empty graph pattern",
        span: clause.span,
    })?;
    Ok(LoweredClause {
        tree,
        names,
        filters,
    })
}

fn lower_graph_pattern(
    pattern: &GraphPattern,
    ctx: &mut GraphLoweringContext<'_, '_>,
) -> Result<(JoinTree, BTreeSet<IStr>), PlannerError> {
    if let Some(name) = pattern.path_binding {
        let binding = binding_for_decl(
            name,
            pattern.span,
            BindingDeclKind::PathBinding,
            ctx.analyzed,
        )?;
        ctx.binding_ids.insert(binding);
        ctx.paths.push(PathPlan {
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
        ctx.analyzed,
        ctx.filters,
        &mut names,
        ctx.binding_ids,
        ctx.hidden,
    )?);
    let source_binding = chain_tail_binding(&current).ok_or(PlannerError::NotImplemented {
        feature: "path selector over bindingless source node",
        span: first.span,
    })?;
    while let Some(element) = elements.next() {
        let PatternElement::Edge(edge) = element else {
            return Err(PlannerError::NotImplemented {
                feature: "non-alternating graph pattern",
                span: pattern.span,
            });
        };
        let Some(PatternElement::Node(right)) = elements.next() else {
            return Err(PlannerError::NotImplemented {
                feature: "edge without target",
                span: edge.span,
            });
        };
        let left_binding = chain_tail_binding(&current);
        let right_node = right_node_predicates(
            right,
            ctx.analyzed,
            ctx.filters,
            &mut names,
            ctx.binding_ids,
            ctx.hidden,
        )?;
        let path_mode = ctx.path_mode;
        let selector = ctx.selector;
        let mut edge_ctx = EdgeLoweringContext {
            analyzed: ctx.analyzed,
            filters: ctx.filters,
            names: &mut names,
            binding_ids: ctx.binding_ids,
            hidden: ctx.hidden,
        };
        current = match &edge.quantifier {
            Some(Quantifier::GraphPattern {
                min,
                max: Some(max),
            }) => {
                repeat::ensure_within_max_quantifier(*max, edge.span)?;
                let mut repeat_edge =
                    repeat::edge_match(edge, left_binding, right_node, &mut edge_ctx)?;
                if repeat_edge.group_binding.is_none()
                    && selector_needs_repeat_group(selector, *min, *max)
                {
                    repeat_edge.group_hidden_binding = Some(ctx.hidden.next());
                }
                JoinTree::Repeat {
                    child: Box::new(current),
                    direction: edge.direction,
                    edge: repeat_edge,
                    min: *min,
                    max: Some(*max),
                    path_mode,
                    selector: None,
                }
            }
            Some(Quantifier::GraphPattern { max: None, .. }) => {
                return Err(PlannerError::NotImplemented {
                    feature: "unbounded variable-length edge patterns",
                    span: edge.span,
                });
            }
            Some(Quantifier::Questioned) => {
                return Err(PlannerError::NotImplemented {
                    feature: "questioned edge quantifier (?)",
                    span: edge.span,
                });
            }
            None => {
                let edge_match = edge_match(edge, left_binding, right_node, &mut edge_ctx)?;
                JoinTree::Expand {
                    child: Box::new(current),
                    direction: edge.direction,
                    edge: edge_match,
                }
            }
        };
    }
    if let Some(selector) = ctx.selector {
        current =
            path_search::wrap_in_path_search(current, selector, source_binding, pattern.span)?;
    }
    Ok((current, names))
}

pub(super) fn right_node_predicates(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    names: &mut BTreeSet<IStr>,
    binding_ids: &mut BTreeSet<BindingId>,
    hidden: &mut HiddenAllocator,
) -> Result<RightNode, PlannerError> {
    let binding = node_binding(node, analyzed, names, binding_ids)?;
    let hidden_binding = binding.is_none().then(|| hidden.next());
    let property_predicates = node
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(binding, *key, value, analyzed))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(where_clause) = &node.inline_where {
        // Why: inline WHERE on an expanded right node may reference bindings
        // outside the edge, so it stays in the pattern-level filter list
        // rather than riding the EdgeMatch.
        filters.push(expr::filter_predicate(where_clause, analyzed)?);
    }
    Ok(RightNode {
        binding,
        hidden_binding,
        label_predicate: node.label_expr.clone(),
        property_predicates,
    })
}

fn node_scan(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    names: &mut BTreeSet<IStr>,
    binding_ids: &mut BTreeSet<BindingId>,
    hidden: &mut HiddenAllocator,
) -> Result<NodeOrEdgeScan, PlannerError> {
    let binding = node_binding(node, analyzed, names, binding_ids)?;
    let hidden_binding = binding.is_none().then(|| hidden.next());
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
        hidden_binding,
        kind: ScanKind::Node,
        label_predicate: node.label_expr.clone(),
        property_predicates,
        access: ScanAccess::Linear,
        span: node.span,
    })
}

fn edge_match(
    edge: &EdgePattern,
    left_binding: Option<TailBinding>,
    right_node: RightNode,
    ctx: &mut EdgeLoweringContext<'_, '_>,
) -> Result<EdgeMatch, PlannerError> {
    let binding = edge_binding(edge, ctx.analyzed, ctx.names, ctx.binding_ids)?;
    let hidden_binding = binding.is_none().then(|| ctx.hidden.next());
    let property_predicates = edge
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(binding, *key, value, ctx.analyzed))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(where_clause) = &edge.inline_where {
        ctx.filters
            .push(expr::filter_predicate(where_clause, ctx.analyzed)?);
    }
    Ok(EdgeMatch {
        binding,
        hidden_binding,
        label_predicate: edge.label_expr.clone(),
        property_predicates,
        left_binding: left_binding.and_then(TailBinding::named),
        left_hidden_binding: left_binding.and_then(TailBinding::hidden),
        right_binding: right_node.binding,
        right_hidden_binding: right_node.hidden_binding,
        right_label_predicate: right_node.label_predicate,
        right_property_predicates: right_node.property_predicates,
        access: ScanAccess::Linear,
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

pub(super) fn edge_binding(
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

fn split_optional_filters(
    filters: Vec<FilterPredicate>,
    left_names: &BTreeSet<IStr>,
    analyzed: &AnalyzedStatement,
) -> (Vec<FilterPredicate>, Vec<FilterPredicate>) {
    let mut right_filters = Vec::new();
    let mut global_filters = Vec::new();
    for filter in filters {
        if references_optional_binding(&filter, left_names, analyzed) {
            right_filters.push(filter);
        } else {
            global_filters.push(filter);
        }
    }
    (right_filters, global_filters)
}

fn references_optional_binding(
    filter: &FilterPredicate,
    left_names: &BTreeSet<IStr>,
    analyzed: &AnalyzedStatement,
) -> bool {
    filter.binding_refs.iter().any(|binding| {
        binding_name(*binding, analyzed).is_some_and(|name| !left_names.contains(&name))
    })
}

fn binding_name(binding: BindingId, analyzed: &AnalyzedStatement) -> Option<IStr> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.id() == binding)
        .map(BindingDecl::name)
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

fn selector_needs_repeat_group(selector: Option<PathSelector>, min: u32, max: u32) -> bool {
    selector.is_some_and(|selector| {
        min != max
            || matches!(
                selector,
                PathSelector::AllShortest | PathSelector::AnyShortest
            )
    })
}

fn shared_names(left: &BTreeSet<IStr>, right: &BTreeSet<IStr>) -> Vec<IStr> {
    left.intersection(right).copied().collect()
}

/// Return the binding of the most-recently expanded chain tail, propagating
/// `None` when the trailing element is anonymous so the caller does not
/// silently fall back to an older named node from earlier in the chain.
fn chain_tail_binding(tree: &JoinTree) -> Option<TailBinding> {
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
            chain_tail_binding(right)
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            intersection.first().and_then(chain_tail_binding)
        }
        JoinTree::Subplan(_) => None,
    }
}
