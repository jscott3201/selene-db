//! MATCH-clause lowering.

use std::collections::BTreeSet;

use selene_core::DbString;

use crate::{
    EdgePattern, GraphPattern, LabelExpr, MatchClause, MatchMode, NodePattern, PathMode,
    PathSelector, PatternElement, Quantifier,
    analyze::{AnalyzedStatement, BindingDecl, BindingDeclKind, BindingId, BindingUseKind},
    plan::{
        BindingDef, BindingElement, BuildSide, EdgeMatch, FilterPredicate, HiddenBindingId,
        JoinTree, NodeOrEdgeScan, PathPlan, PatternPlan, PlannerError, ScanAccess, ScanKind,
        TailBinding,
    },
};

struct LoweredClause {
    tree: JoinTree,
    names: BTreeSet<DbString>,
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
    /// Embedder-configured variable-length quantifier upper-bound cap
    /// (`ImplDefinedCaps::max_quantifier`), threaded to the plan-time gate.
    max_quantifier: u32,
    /// True when this MATCH carries a DIFFERENT EDGES (`G002`) match mode. Per
    /// ISO 39075:2024 §16.4 NOTE 222, DIFFERENT EDGES imparts the effect of
    /// TRAIL to each path pattern, so every quantified edge must (a) carry an
    /// edge-identity group slot the pattern-wide filter can read and (b) prune
    /// repeated edges *during* an unbounded repeat — otherwise the post-walk
    /// filter never runs because the WALK traversal first hits `max_quantifier`.
    different_edges: bool,
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
    pub(super) names: &'s mut BTreeSet<DbString>,
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

use super::{expr, match_mode, path_mode, path_search, repeat};

/// Lower leading MATCH clauses into one pattern plan.
pub(crate) fn lower_match_prefix(
    clauses: &[&MatchClause],
    analyzed: &AnalyzedStatement,
    max_quantifier: u32,
) -> Result<Option<PatternPlan>, PlannerError> {
    if clauses.is_empty() {
        return Ok(None);
    }

    let mut filters = Vec::new();
    let mut paths = Vec::new();
    let mut binding_ids = BTreeSet::new();
    let mut hidden = HiddenAllocator::default();
    let mut current: Option<(JoinTree, BTreeSet<DbString>)> = None;

    for clause in clauses {
        reject_unsupported_clause(clause)?;
        let lowered = lower_match_clause(
            clause,
            analyzed,
            &mut paths,
            &mut binding_ids,
            &mut hidden,
            max_quantifier,
        )?;
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

pub(super) fn lower_pipeline_match(
    clause: &MatchClause,
    analyzed: &AnalyzedStatement,
    left_names: &BTreeSet<DbString>,
    max_quantifier: u32,
) -> Result<(PatternPlan, Vec<FilterPredicate>), PlannerError> {
    reject_unsupported_clause(clause)?;
    let mut paths = Vec::new();
    let mut binding_ids = BTreeSet::new();
    let mut hidden = HiddenAllocator::default();
    let lowered = lower_match_clause(
        clause,
        analyzed,
        &mut paths,
        &mut binding_ids,
        &mut hidden,
        max_quantifier,
    )?;
    let (filters, global_filters) = if clause.optional {
        split_optional_filters(lowered.filters, left_names, analyzed)
    } else {
        (lowered.filters, Vec::new())
    };
    Ok((
        PatternPlan {
            bindings: binding_defs(analyzed, &binding_ids),
            join_tree: lowered.tree,
            filters,
            paths,
        },
        global_filters,
    ))
}

fn lower_match_clause(
    clause: &MatchClause,
    analyzed: &AnalyzedStatement,
    paths: &mut Vec<PathPlan>,
    binding_ids: &mut BTreeSet<BindingId>,
    hidden: &mut HiddenAllocator,
    max_quantifier: u32,
) -> Result<LoweredClause, PlannerError> {
    let mut filters = Vec::new();
    let mut current: Option<(JoinTree, BTreeSet<DbString>)> = None;
    for pattern in &clause.patterns {
        let mut ctx = GraphLoweringContext {
            path_mode: clause.path_mode,
            selector: clause.selector,
            analyzed,
            filters: &mut filters,
            paths,
            binding_ids,
            hidden,
            max_quantifier,
            different_edges: clause.match_mode == Some(MatchMode::DifferentEdges),
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
    // Per ISO 39075:2024 §16.4: the `<match mode>` is pattern-wide, so it wraps
    // the join of every comma-separated path pattern in this MATCH clause (the
    // `current` tree above). DIFFERENT EDGES installs the pattern-wide
    // edge-uniqueness filter here; REPEATABLE ELEMENTS and the ID086 default
    // install nothing.
    let tree = match_mode::wrap_in_match_mode_filter(tree, clause.match_mode, clause.span)?;
    Ok(LoweredClause {
        tree,
        names,
        filters,
    })
}

fn lower_graph_pattern(
    pattern: &GraphPattern,
    ctx: &mut GraphLoweringContext<'_, '_>,
) -> Result<(JoinTree, BTreeSet<DbString>), PlannerError> {
    if let Some(name) = &pattern.path_binding {
        let binding = binding_for_decl(
            name.clone(),
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
    // Capture the source node's binding from the initial scan (before the loop
    // reassigns `current`). The bindingless-source error is only meaningful for
    // a path *selector*, so it is deferred to the `if let Some(selector)` block
    // below — a non-selector pattern over a bindingless source is legal.
    let source_binding = chain_tail_binding(&current);
    // Per ISO 39075:2024 §16.4 NOTE 222, DIFFERENT EDGES imparts TRAIL to each
    // path pattern. When a path SELECTOR (ANY/SHORTEST) is present it ranks/picks
    // paths BEFORE the outer pattern-wide `MatchModeFilter` runs, so a bare-WALK
    // repeat could surface an edge-reusing path that the selector chooses and the
    // filter then drops — losing a valid edge-distinct binding. Bumping the
    // effective path mode to TRAIL installs the per-path trail filter
    // (`wrap_in_path_mode_filter`) BENEATH the selector, so the selector only
    // ever ranks edge-distinct paths. Non-selector patterns keep their declared
    // mode: the pattern-wide filter is then the sole, correct authority
    // (result-equivalent, and it avoids a redundant per-path trail pass).
    let effective_path_mode =
        if ctx.different_edges && ctx.selector.is_some() && ctx.path_mode == PathMode::Walk {
            PathMode::Trail
        } else {
            ctx.path_mode
        };
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
        let path_mode = effective_path_mode;
        let selector = ctx.selector;
        let max_quantifier = ctx.max_quantifier;
        let different_edges = ctx.different_edges;
        let mut edge_ctx = EdgeLoweringContext {
            analyzed: ctx.analyzed,
            filters: ctx.filters,
            names: &mut names,
            binding_ids: ctx.binding_ids,
            hidden: ctx.hidden,
        };
        current = match &edge.quantifier {
            Some(Quantifier::GraphPattern { min, max }) => {
                if let Some(max) = max {
                    repeat::ensure_within_max_quantifier(*max, max_quantifier, edge.span)?;
                }
                let mut repeat_edge =
                    repeat::edge_match(edge, left_binding, right_node, &mut edge_ctx)?;
                if repeat_edge.group_binding.is_none()
                    && repeat_needs_hidden_group(selector, path_mode, *min, *max, different_edges)
                {
                    repeat_edge.group_hidden_binding = Some(ctx.hidden.next());
                }
                JoinTree::Repeat {
                    child: Box::new(current),
                    direction: edge.direction,
                    edge: repeat_edge,
                    min: *min,
                    max: *max,
                    path_mode: repeat_path_mode_under_filter(
                        path_mode,
                        *min,
                        *max,
                        different_edges,
                        selector,
                    ),
                }
            }
            Some(Quantifier::Questioned) => {
                let edge_match = edge_match(edge, left_binding, right_node, &mut edge_ctx)?;
                let source_binding = left_binding.ok_or(PlannerError::NotImplemented {
                    feature: "questioned edge without source node binding",
                    span: edge.span,
                })?;
                let final_binding = edge_match
                    .right_binding
                    .map(TailBinding::Named)
                    .or_else(|| edge_match.right_hidden_binding.map(TailBinding::Hidden))
                    .ok_or(PlannerError::NotImplemented {
                        feature: "questioned edge without final node binding",
                        span: edge.span,
                    })?;
                JoinTree::Questioned {
                    child: Box::new(current),
                    direction: edge.direction,
                    edge: edge_match,
                    source_binding,
                    final_binding,
                }
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
    current = path_mode::wrap_in_path_mode_filter(current, effective_path_mode, pattern.span)?;
    if let Some(selector) = ctx.selector {
        let source_binding = source_binding.ok_or(PlannerError::NotImplemented {
            feature: "path selector over bindingless source node",
            span: first.span,
        })?;
        current =
            path_search::wrap_in_path_search(current, selector, source_binding, pattern.span)?;
    }
    Ok((current, names))
}

pub(super) fn right_node_predicates(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
    filters: &mut Vec<FilterPredicate>,
    names: &mut BTreeSet<DbString>,
    binding_ids: &mut BTreeSet<BindingId>,
    hidden: &mut HiddenAllocator,
) -> Result<RightNode, PlannerError> {
    let binding = node_binding(node, analyzed, names, binding_ids)?;
    let hidden_binding = binding.is_none().then(|| hidden.next());
    let property_predicates = node
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(binding, key.clone(), value, analyzed))
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
    names: &mut BTreeSet<DbString>,
    binding_ids: &mut BTreeSet<BindingId>,
    hidden: &mut HiddenAllocator,
) -> Result<NodeOrEdgeScan, PlannerError> {
    let binding = node_binding(node, analyzed, names, binding_ids)?;
    let hidden_binding = binding.is_none().then(|| hidden.next());
    let property_predicates = node
        .properties
        .iter()
        .map(|(key, value)| expr::property_predicate(binding, key.clone(), value, analyzed))
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
        .map(|(key, value)| expr::property_predicate(binding, key.clone(), value, ctx.analyzed))
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
    names: &mut BTreeSet<DbString>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<Option<BindingId>, PlannerError> {
    node.binding
        .clone()
        .map(|name| {
            names.insert(name.clone());
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
    names: &mut BTreeSet<DbString>,
    binding_ids: &mut BTreeSet<BindingId>,
) -> Result<Option<BindingId>, PlannerError> {
    edge.binding
        .clone()
        .map(|name| {
            names.insert(name.clone());
            let binding =
                binding_for_pattern(name, edge.span, BindingDeclKind::EdgePattern, analyzed)?;
            binding_ids.insert(binding);
            Ok(binding)
        })
        .transpose()
}

fn binding_for_pattern(
    name: DbString,
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
    name: DbString,
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
    left_names: &BTreeSet<DbString>,
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
    left_names: &BTreeSet<DbString>,
    analyzed: &AnalyzedStatement,
) -> bool {
    filter.binding_refs.iter().any(|binding| {
        binding_name(*binding, analyzed).is_some_and(|name| !left_names.contains(&name))
    })
}

fn binding_name(binding: BindingId, analyzed: &AnalyzedStatement) -> Option<DbString> {
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
                | BindingDeclKind::ForAlias
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
    // Why: true backstop against any future `<match mode>` (ISO 39075:2024
    // §16.4) that reaches the planner without a lowering arm. Per the §16.4
    // Conformance Rules, G002 (DIFFERENT EDGES) and G003 (REPEATABLE ELEMENTS)
    // are both claimed and lowered, so this guard lets them pass; it errors only
    // on an unhandled mode so a future register change (registering a new mode
    // without wiring its runtime contract) cannot silently lower it. The match
    // is exhaustive so the compiler forces an explicit decision for any added
    // `MatchMode` variant.
    match clause.match_mode {
        // Per ISO 39075:2024 §16.4 GR8: DIFFERENT EDGES installs the pattern-wide
        // edge-uniqueness filter at lowering; REPEATABLE ELEMENTS installs none
        // (GR8(b): BINDINGS = INNER). Both are handled — no rejection.
        None | Some(MatchMode::DifferentEdges) | Some(MatchMode::RepeatableElements) => Ok(()),
    }
}

fn repeat_needs_hidden_group(
    selector: Option<PathSelector>,
    path_mode: PathMode,
    min: u32,
    max: Option<u32>,
    different_edges: bool,
) -> bool {
    // Per ISO 39075:2024 §16.4 NOTE 222: a DIFFERENT EDGES match mode imparts
    // TRAIL to every path pattern, so an *anonymous* quantified edge still needs
    // an edge-identity group slot — the pattern-wide `MatchModeFilter` reads it
    // via `collect_path_contributors`/`repeat_contributor`. Without this, a
    // legal G002 pattern such as `MATCH DIFFERENT EDGES (a)-[:K*2]->(b)` would
    // fail to plan ("path mode over quantified edge without edge group slot").
    different_edges
        || path_mode != PathMode::Walk
        || selector.is_some_and(|selector| selector_needs_repeat_group(selector, min, max))
}

fn selector_needs_repeat_group(selector: PathSelector, min: u32, max: Option<u32>) -> bool {
    // Per ISO 39075:2024 §22.4: every shortest-based selector (ALL/ANY SHORTEST
    // plus the counted G019/G020 forms) ranks bindings by hop count, so each
    // needs the hidden hop-count group slot even at a fixed length.
    max != Some(min)
        || matches!(
            selector,
            PathSelector::AllShortest
                | PathSelector::AnyShortest
                | PathSelector::CountedShortest { .. }
                | PathSelector::CountedShortestGroup { .. }
        )
}

fn repeat_path_mode_under_filter(
    path_mode: PathMode,
    min: u32,
    max: Option<u32>,
    different_edges: bool,
    selector: Option<PathSelector>,
) -> PathMode {
    if max.is_none() {
        // An unbounded repeat must prune *during* traversal or it never
        // terminates on a cyclic graph. Per ISO 39075:2024 §16.4 NOTE 222, a
        // DIFFERENT EDGES match mode imparts TRAIL to each path pattern, so a
        // bare WALK quantifier under DIFFERENT EDGES traverses as TRAIL — this
        // is exactly the finiteness guarantee the analyzer relies on when it
        // admits an unbounded quantifier gated only by DIFFERENT EDGES
        // (`analyze::bind::pattern::validate_unbounded_legality`). A more
        // restrictive declared path mode (TRAIL/ACYCLIC/SIMPLE) already prunes
        // and is the user's explicit per-path choice, so it is left intact; the
        // pattern-wide post-walk filter then enforces the broader G002
        // cross-path-pattern edge-uniqueness on top.
        //
        // A minimum-length shortest selector — `ANY SHORTEST` / `ALL SHORTEST`
        // and their ISO §16.6 SR2c-equivalent count-1 counted spellings
        // `SHORTEST 1 [PATH]` / `SHORTEST [1] GROUP[S]` — gives the same
        // finiteness guarantee and the same downshift is result-equivalent —
        // BUT ONLY when the quantifier lower bound is <= 1.
        // Reason: an UNCONSTRAINED minimum-hop path never repeats a node (a
        // repeated node is a removable cycle, yielding a strictly shorter path), so
        // it is simple, hence a trail; TRAIL traversal then contains *all*
        // minimum-hop paths (the extra node-repeating trails it produces are
        // strictly longer and the shortest selector discards them). With a lower
        // bound `min >= 2` that argument FAILS: removing the cycle would drop below
        // `min`, so the shortest path satisfying the bound can legitimately reuse an
        // edge — e.g. `ALL SHORTEST WALK (a)-[r:K*2..]->(a)` over a self-loop has
        // shortest walk `[e, e]`, which TRAIL would reject (losing the row). So the
        // shortest downshift is gated on `min <= 1`; a `min >= 2` shortest stays
        // WALK and, over a cyclic graph, raises 5GQL1 (deferred — like counted, it
        // needs ordered length-enumeration over an infinite WALK candidate set).
        //
        // The count-`>= 2` counted forms (`SHORTEST N` G019 / `SHORTEST N GROUP`
        // G020, N >= 2) are deliberately *excluded* regardless of bound: per ISO
        // 39075:2024 §22.4 they rank paths by hop count *including* non-simple
        // (cyclic) paths — consistent with selene's bounded counted-shortest.
        // Downshifting them to TRAIL would silently change their semantics (count
        // trails, not walks); over an unbounded cyclic WALK the candidate set is
        // infinite, so plain counted (N >= 2) stays WALK and raises 5GQL1. The
        // count-`1` forms are NOT excluded — they ARE the min-length shortest
        // selector above (§16.6 SR2c) and downshift identically to ANY/ALL
        // SHORTEST.
        // NOTE the exception: a selector written WITH `DIFFERENT EDGES` *does*
        // downshift via the `different_edges` arm regardless of `min` / counted, and
        // that is correct — DIFFERENT EDGES constrains the candidate set to
        // edge-distinct paths (TRAIL), which is finite, so e.g.
        // `SHORTEST N DIFFERENT EDGES` counts the N shortest edge-distinct paths and
        // terminates. The min/counted exclusions above are only about the *plain*
        // (WALK) forms.
        let shortest_simple_safe = is_min_length_shortest(selector) && min <= 1;
        if path_mode == PathMode::Walk && (different_edges || shortest_simple_safe) {
            PathMode::Trail
        } else {
            path_mode
        }
    } else {
        // Bounded repeats are finite under WALK; the post-walk filter (path-mode
        // or pattern-wide match-mode) prunes the surviving rows.
        PathMode::Walk
    }
}

/// A minimum-length shortest selector retains *only* the single minimum
/// hop-rank, so the TRAIL downshift is result-equivalent (every minimum-hop
/// path is simple when the lower bound is <= 1). Per ISO 39075:2024 §16.6 SR2c
/// the count-1 counted forms are the *same selector* as the keyword spellings —
/// `ANY SHORTEST == SHORTEST 1 [PATH]` (`CountedShortest { paths: 1 }`) and
/// `ALL SHORTEST == SHORTEST [1] GROUP[S]` (`CountedShortestGroup { groups: 1 }`,
/// the count defaulting to 1 per §16.6 SR2b) — the runtime collapses all four to
/// one `select_counted(count = 1, group)` (`runtime::path_search`). Matching on
/// the count-1 *semantics* (not just the keyword spelling) keeps ISO-equivalent
/// forms behaving identically on cyclic graphs.
///
/// The count-`>= 2` counted forms (`SHORTEST N` / `SHORTEST N GROUP[S]`, N >= 2)
/// are intentionally excluded: they admit longer, possibly non-simple, paths
/// (§22.4), so the TRAIL downshift is not result-equivalent for them. See
/// `repeat_path_mode_under_filter`.
fn is_min_length_shortest(selector: Option<PathSelector>) -> bool {
    matches!(
        selector,
        Some(
            PathSelector::AnyShortest
                | PathSelector::AllShortest
                | PathSelector::CountedShortest { paths: 1 }
                | PathSelector::CountedShortestGroup { groups: 1 }
        )
    )
}

fn shared_names(left: &BTreeSet<DbString>, right: &BTreeSet<DbString>) -> Vec<DbString> {
    left.intersection(right).cloned().collect()
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
        JoinTree::Questioned { final_binding, .. } => Some(*final_binding),
        JoinTree::Repeat { edge, .. } => edge
            .final_binding
            .map(TailBinding::Named)
            .or_else(|| edge.final_hidden_binding.map(TailBinding::Hidden)),
        JoinTree::PathModeFilter { child, .. } | JoinTree::MatchModeFilter { child, .. } => {
            chain_tail_binding(child)
        }
        JoinTree::PathSearch { final_binding, .. } => Some(*final_binding),
        JoinTree::HashJoin { right, .. } | JoinTree::Outer { right, .. } => {
            chain_tail_binding(right)
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            intersection.first().and_then(chain_tail_binding)
        }
        JoinTree::Subplan(_) => None,
        JoinTree::DisjunctiveScan { .. } => {
            // DisjunctiveScan is emitted by the disjunctive_label_expansion
            // optimizer rule (post-lowering); chain_tail_binding only runs
            // during MATCH-clause lowering, before any optimizer rule fires.
            unreachable!("DisjunctiveScan is rule-emitted post-lowering")
        }
    }
}
