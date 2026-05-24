//! Shared optimizer walkers.

use crate::{
    IsCheckKind, PatternElement, StatementCategory, ValueExpr,
    plan::{
        BindingDef, BindingTableSchema, CatalogOp, EdgeMatch, ExecutionPlan, FilterPredicate,
        FilterPredicateKind, JoinTree, MutationOp, OrderKey, PipelineOp,
        PlannedTypePropertyConstraint, ProjectExpr, PropertyInit, Transformed,
        optimize::{OptimizeContext, Rule, binding_refs},
    },
};

/// Visit every expression-bearing IR site in one plan, excluding nested
/// subplans reached through `Union`, `Chain`, and `JoinTree::Subplan`.
///
/// Expression-bearing rows that carry `binding_refs` are refreshed after a
/// mutation when their binding context can resolve every referenced variable.
pub(crate) fn walk_value_exprs(
    plan: &mut ExecutionPlan,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let mut changed = false;
    if let Some(pattern) = &mut plan.pattern_plan {
        changed |= walk_predicates(&mut pattern.filters, &pattern.bindings, visit);
        changed |= walk_join_tree_exprs(&mut pattern.join_tree, &pattern.bindings, visit);
    }
    for op in &mut plan.pipeline {
        // Pipeline aliases do not currently carry enough binding metadata to
        // rebuild a row context by name here. Preserve existing refs on any
        // variable-bearing expression and refresh literal-only rows to empty.
        changed |= walk_pipeline_op_exprs(op, &[], visit);
    }
    changed
}

/// Walk a [`FilterPredicate`] expression and refresh its `binding_refs` when
/// the visitor mutates it.
pub(crate) fn walk_and_sync_binding_refs_filter(
    predicate: &mut FilterPredicate,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let changed = walk_expr(&mut predicate.expr, visit);
    if changed {
        sync_filter_binding_refs(predicate, bindings);
    }
    changed
}

/// Walk a [`ProjectExpr`] expression and refresh its `binding_refs` when the
/// visitor mutates it.
pub(crate) fn walk_and_sync_binding_refs_project(
    project: &mut ProjectExpr,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let changed = walk_expr(&mut project.expr, visit);
    if changed {
        sync_binding_refs(
            "ProjectExpr",
            &project.expr,
            &mut project.binding_refs,
            bindings,
        );
    }
    changed
}

/// Walk an [`OrderKey`] expression and refresh its `binding_refs` when the
/// visitor mutates it.
pub(crate) fn walk_and_sync_binding_refs_order(
    order: &mut OrderKey,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let changed = walk_expr(&mut order.expr, visit);
    if changed {
        sync_binding_refs("OrderKey", &order.expr, &mut order.binding_refs, bindings);
    }
    changed
}

/// Walk a [`PropertyInit`] value expression and refresh its `binding_refs` when
/// the visitor mutates it.
pub(crate) fn walk_and_sync_binding_refs_property_init(
    init: &mut PropertyInit,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let changed = walk_expr(&mut init.value.expr, visit);
    if changed {
        sync_binding_refs(
            "PropertyInit",
            &init.value.expr,
            &mut init.value.binding_refs,
            bindings,
        );
    }
    changed
}

/// Recurse into nested execution plans reached by pipeline or join-tree
/// subplan boundaries.
pub(crate) fn recurse_subplans(
    mut plan: ExecutionPlan,
    visit: &mut impl FnMut(ExecutionPlan) -> Transformed<ExecutionPlan>,
) -> Transformed<ExecutionPlan> {
    let mut changed = false;
    if let Some(pattern) = &mut plan.pattern_plan {
        changed |= recurse_join_tree_subplans(&mut pattern.join_tree, visit);
    }
    for op in &mut plan.pipeline {
        match op {
            PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
                changed |= recurse_plan_box(rhs, visit);
            }
            PipelineOp::CallSubquery(subquery) => {
                changed |= recurse_plan_box(&mut subquery.body, visit);
            }
            PipelineOp::ExplainPlan { inner, .. } => changed |= recurse_plan_box(inner, visit),
            PipelineOp::Filter(_)
            | PipelineOp::Project(_)
            | PipelineOp::Let(_)
            | PipelineOp::Unwind { .. }
            | PipelineOp::OrderBy(_)
            | PipelineOp::Limit { .. }
            | PipelineOp::TopK { .. }
            | PipelineOp::GroupBy { .. }
            | PipelineOp::Distinct
            | PipelineOp::Match(_)
            | PipelineOp::OptionalMatch(_)
            | PipelineOp::Call(_)
            | PipelineOp::Mutation(_)
            | PipelineOp::Catalog(_)
            | PipelineOp::Tx(_) => {}
        }
    }
    Transformed { plan, changed }
}

/// Recurse into nested plans with the same rule and context.
pub(crate) fn recurse_rule_subplans<R>(
    plan: ExecutionPlan,
    rule: &R,
    ctx: &OptimizeContext<'_>,
) -> Transformed<ExecutionPlan>
where
    R: Rule + ?Sized,
{
    recurse_subplans(plan, &mut |subplan| rule.rewrite(subplan, ctx))
}

/// Visit `Expand` edges that are safe targets for filter pushdown.
///
/// Excludes WCO and `Subplan` boundaries (rules don't reach across those),
/// and the optional-side subtree of any `JoinTree::Outer`.
///
/// Why: pushing a single-binding predicate into an `Expand` edge under
/// `JoinTree::Outer.right` evaluates it before null-extension, dropping
/// rows that a post-OPTIONAL `FILTER` would have null-extended and kept.
/// Recursing only into `Outer.left` preserves the rule's preserved-side
/// pushdown opportunities while leaving optional-side filters in
/// `pattern.filters`, where they correctly run after null-extension.
pub(crate) fn walk_expand_nodes(
    tree: &mut JoinTree,
    visit: &mut impl FnMut(&mut EdgeMatch) -> bool,
) -> bool {
    match tree {
        JoinTree::Scan(_)
        | JoinTree::Repeat { .. }
        | JoinTree::Questioned { .. }
        | JoinTree::PathSearch { .. }
        | JoinTree::PathModeFilter { .. }
        | JoinTree::WorstCaseOptimal { .. }
        | JoinTree::Subplan(_) => false,
        JoinTree::Expand { child, edge, .. } => {
            let changed_child = walk_expand_nodes(child, visit);
            visit(edge) | changed_child
        }
        JoinTree::HashJoin { left, right, .. } => {
            walk_expand_nodes(left, visit) | walk_expand_nodes(right, visit)
        }
        JoinTree::Outer { left, .. } => walk_expand_nodes(left, visit),
    }
}

fn recurse_join_tree_subplans(
    tree: &mut JoinTree,
    visit: &mut impl FnMut(ExecutionPlan) -> Transformed<ExecutionPlan>,
) -> bool {
    match tree {
        JoinTree::Scan(_) | JoinTree::WorstCaseOptimal { .. } => false,
        JoinTree::Expand { child, .. }
        | JoinTree::Questioned { child, .. }
        | JoinTree::Repeat { child, .. }
        | JoinTree::PathSearch { child, .. }
        | JoinTree::PathModeFilter { child, .. } => recurse_join_tree_subplans(child, visit),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            recurse_join_tree_subplans(left, visit) | recurse_join_tree_subplans(right, visit)
        }
        JoinTree::Subplan(plan) => recurse_plan_box(plan, visit),
    }
}

fn recurse_plan_box(
    plan: &mut Box<ExecutionPlan>,
    visit: &mut impl FnMut(ExecutionPlan) -> Transformed<ExecutionPlan>,
) -> bool {
    let current = std::mem::replace(plan, Box::new(empty_plan()));
    let transformed = visit(*current);
    **plan = transformed.plan;
    transformed.changed
}

fn empty_plan() -> ExecutionPlan {
    ExecutionPlan {
        category: StatementCategory::ReadOnly,
        pattern_plan: None,
        pipeline: Vec::new(),
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: Default::default(),
        expr_ids: Default::default(),
        subqueries: Default::default(),
        next_expr_id: crate::ExprId::new(0),
        next_pipeline_op_id: crate::PipelineOpId::new(0),
    }
}

fn walk_join_tree_exprs(
    tree: &mut JoinTree,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match tree {
        JoinTree::Scan(scan) => walk_predicates(&mut scan.property_predicates, bindings, visit),
        JoinTree::Expand { child, edge, .. } => {
            let changed_child = walk_join_tree_exprs(child, bindings, visit);
            let changed_edge = walk_predicates(&mut edge.property_predicates, bindings, visit)
                | walk_predicates(&mut edge.right_property_predicates, bindings, visit);
            changed_child | changed_edge
        }
        JoinTree::Questioned { child, edge, .. } => {
            let changed_child = walk_join_tree_exprs(child, bindings, visit);
            let changed_edge = walk_predicates(&mut edge.property_predicates, bindings, visit)
                | walk_predicates(&mut edge.right_property_predicates, bindings, visit);
            changed_child | changed_edge
        }
        JoinTree::Repeat { child, edge, .. } => {
            let changed_child = walk_join_tree_exprs(child, bindings, visit);
            let changed_edge = walk_predicates(&mut edge.property_predicates, bindings, visit)
                | walk_predicates(&mut edge.inline_predicates, bindings, visit)
                | walk_predicates(&mut edge.final_property_predicates, bindings, visit);
            changed_child | changed_edge
        }
        JoinTree::PathSearch { child, .. } | JoinTree::PathModeFilter { child, .. } => {
            walk_join_tree_exprs(child, bindings, visit)
        }
        JoinTree::HashJoin { left, right, .. } => {
            walk_join_tree_exprs(left, bindings, visit)
                | walk_join_tree_exprs(right, bindings, visit)
        }
        JoinTree::Outer {
            left,
            right,
            right_filters,
            ..
        } => {
            walk_join_tree_exprs(left, bindings, visit)
                | walk_join_tree_exprs(right, bindings, visit)
                | walk_predicates(right_filters, bindings, visit)
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => false,
    }
}

fn walk_pipeline_op_exprs(
    op: &mut PipelineOp,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match op {
        PipelineOp::Filter(pred) => walk_and_sync_binding_refs_filter(pred, bindings, visit),
        PipelineOp::Project(items) | PipelineOp::Let(items) => {
            items.iter_mut().fold(false, |changed, item| {
                walk_and_sync_binding_refs_project(item, bindings, visit) | changed
            })
        }
        PipelineOp::Unwind { source, .. } => {
            walk_and_sync_binding_refs_project(source, bindings, visit)
        }
        PipelineOp::OrderBy(keys) => keys.iter_mut().fold(false, |changed, key| {
            walk_and_sync_binding_refs_order(key, bindings, visit) | changed
        }),
        PipelineOp::TopK { keys, .. } => keys.iter_mut().fold(false, |changed, key| {
            walk_and_sync_binding_refs_order(key, bindings, visit) | changed
        }),
        PipelineOp::GroupBy { keys, aggregates } => {
            let key_changed = keys.iter_mut().fold(false, |changed, key| {
                walk_and_sync_binding_refs_project(key, bindings, visit) | changed
            });
            let aggregate_changed = aggregates.iter_mut().fold(false, |changed, aggregate| {
                aggregate.args.iter_mut().fold(false, |arg_changed, arg| {
                    walk_expr(&mut arg.expr, visit) | arg_changed
                }) | changed
            });
            key_changed | aggregate_changed
        }
        PipelineOp::Call(call) => call.args.iter_mut().fold(false, |changed, arg| {
            walk_and_sync_binding_refs_project(arg, bindings, visit) | changed
        }),
        PipelineOp::Match(pattern) | PipelineOp::OptionalMatch(pattern) => {
            walk_join_tree_exprs(&mut pattern.join_tree, &pattern.bindings, visit)
        }
        PipelineOp::Mutation(mutation) => walk_mutation_exprs(mutation, bindings, visit),
        PipelineOp::Catalog(catalog) => walk_catalog_exprs(catalog, bindings, visit),
        PipelineOp::Limit { .. }
        | PipelineOp::Distinct
        | PipelineOp::Union { .. }
        | PipelineOp::Chain(_)
        | PipelineOp::CallSubquery(_)
        | PipelineOp::ExplainPlan { .. }
        | PipelineOp::Tx(_) => false,
    }
}

fn walk_mutation_exprs(
    mutation: &mut MutationOp,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match mutation {
        MutationOp::InsertNode { property_inits, .. }
        | MutationOp::InsertEdge { property_inits, .. } => {
            property_inits.iter_mut().fold(false, |changed, init| {
                walk_and_sync_binding_refs_property_init(init, bindings, visit) | changed
            })
        }
        MutationOp::SetProperty { value, .. } => {
            walk_and_sync_binding_refs_project(value, bindings, visit)
        }
        MutationOp::SetLabel { .. }
        | MutationOp::RemoveProperty { .. }
        | MutationOp::RemoveLabel { .. }
        | MutationOp::DeleteTarget { .. } => false,
    }
}

fn walk_catalog_exprs(
    catalog: &mut CatalogOp,
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match catalog {
        CatalogOp::CreateNodeType { properties, .. }
        | CatalogOp::CreateEdgeType { properties, .. } => {
            properties.iter_mut().fold(false, |changed, property| {
                property
                    .constraints
                    .iter_mut()
                    .fold(false, |constraint_changed, constraint| match constraint {
                        PlannedTypePropertyConstraint::Default(expr, _) => {
                            walk_and_sync_binding_refs_project(expr, bindings, visit)
                                | constraint_changed
                        }
                        PlannedTypePropertyConstraint::NotNull(_)
                        | PlannedTypePropertyConstraint::Immutable(_)
                        | PlannedTypePropertyConstraint::Unique(_)
                        | PlannedTypePropertyConstraint::Indexed { .. }
                        | PlannedTypePropertyConstraint::Searchable(_)
                        | PlannedTypePropertyConstraint::Dictionary(_)
                        | PlannedTypePropertyConstraint::Fill(_, _)
                        | PlannedTypePropertyConstraint::Interval(_, _)
                        | PlannedTypePropertyConstraint::Encoding(_, _) => constraint_changed,
                    })
                    | changed
            })
        }
        CatalogOp::CreateGraph { .. }
        | CatalogOp::DropGraph { .. }
        | CatalogOp::DropNodeType { .. }
        | CatalogOp::DropEdgeType { .. }
        | CatalogOp::ShowNodeTypes(_)
        | CatalogOp::ShowEdgeTypes(_)
        | CatalogOp::ShowIndexes(_)
        | CatalogOp::ShowProcedures(_) => false,
    }
}

fn walk_predicates(
    predicates: &mut [FilterPredicate],
    bindings: &[BindingDef],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    predicates.iter_mut().fold(false, |changed, pred| {
        walk_and_sync_binding_refs_filter(pred, bindings, visit) | changed
    })
}

fn sync_filter_binding_refs(predicate: &mut FilterPredicate, bindings: &[BindingDef]) {
    let Some(mut refs) = binding_refs::collect_binding_refs(&predicate.expr, bindings) else {
        tracing::debug!(
            site = "FilterPredicate",
            "optimizer expression rewrite could not recompute binding_refs; leaving existing refs"
        );
        return;
    };
    if let FilterPredicateKind::PropertyEquals {
        binding: Some(binding),
        ..
    } = predicate.kind
    {
        refs.push(binding);
        refs.sort();
        refs.dedup();
    }
    predicate.binding_refs = refs;
}

fn sync_binding_refs(
    site: &'static str,
    expr: &ValueExpr,
    binding_refs: &mut Vec<crate::BindingId>,
    bindings: &[BindingDef],
) {
    if let Some(refs) = binding_refs::collect_binding_refs(expr, bindings) {
        *binding_refs = refs;
    } else {
        tracing::debug!(
            site = site,
            "optimizer expression rewrite could not recompute binding_refs; leaving existing refs"
        );
    }
}

fn walk_expr(expr: &mut ValueExpr, visit: &mut impl FnMut(&mut ValueExpr) -> bool) -> bool {
    let changed_children = match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => false,
        ValueExpr::PropertyAccess { target, .. } => walk_expr(target, visit),
        ValueExpr::ListAccess { target, index, .. } => {
            walk_expr(target, visit) | walk_expr(index, visit)
        }
        ValueExpr::ListLiteral { items, .. } => items
            .iter_mut()
            .fold(false, |changed, item| walk_expr(item, visit) | changed),
        ValueExpr::RecordLiteral { fields, .. } => {
            fields.iter_mut().fold(false, |changed, (_, value)| {
                walk_expr(value, visit) | changed
            })
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => walk_expr(lhs, visit) | walk_expr(rhs, visit),
        ValueExpr::UnaryOp { operand, .. } => walk_expr(operand, visit),
        ValueExpr::FunctionCall { args, .. } => args
            .iter_mut()
            .fold(false, |changed, arg| walk_expr(arg, visit) | changed),
        ValueExpr::Normalize { source, .. } => walk_expr(source, visit),
        ValueExpr::Trim {
            character, source, ..
        } => {
            let character_changed = character
                .as_mut()
                .is_some_and(|character| walk_expr(character, visit));
            character_changed | walk_expr(source, visit)
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            walk_expr(operand, visit) | walk_is_check(kind, visit)
        }
        ValueExpr::InList { operand, list, .. } => {
            walk_expr(operand, visit)
                | list
                    .iter_mut()
                    .fold(false, |changed, item| walk_expr(item, visit) | changed)
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => walk_expr(operand, visit) | walk_expr(pattern, visit),
        ValueExpr::Between {
            operand, low, high, ..
        } => walk_expr(operand, visit) | walk_expr(low, visit) | walk_expr(high, visit),
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => items
            .iter_mut()
            .fold(false, |changed, item| walk_expr(item, visit) | changed),
        ValueExpr::PropertyExists { target, .. } => walk_expr(target, visit),
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            let branch_changed = branches.iter_mut().fold(false, |changed, (when, then)| {
                walk_expr(when, visit) | walk_expr(then, visit) | changed
            });
            let else_changed = else_branch
                .as_mut()
                .is_some_and(|value| walk_expr(value, visit));
            branch_changed | else_changed
        }
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            walk_match_clause(pattern, visit)
        }
        ValueExpr::ValueSubquery { .. } => false,
        ValueExpr::Cast { value, .. } => walk_expr(value, visit),
    };
    visit(expr) | changed_children
}

fn walk_is_check(kind: &mut IsCheckKind, visit: &mut impl FnMut(&mut ValueExpr) -> bool) -> bool {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => walk_expr(value, visit),
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::Normalized(_) => false,
    }
}

fn walk_match_clause(
    clause: &mut crate::MatchClause,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let pattern_changed = clause.patterns.iter_mut().fold(false, |changed, pattern| {
        pattern
            .elements
            .iter_mut()
            .fold(false, |element_changed, element| {
                walk_pattern_element(element, visit) | element_changed
            })
            | changed
    });
    let where_changed = clause
        .where_clause
        .as_mut()
        .is_some_and(|value| walk_expr(value, visit));
    pattern_changed | where_changed
}

fn walk_pattern_element(
    element: &mut PatternElement,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match element {
        PatternElement::Node(node) => {
            let property_changed = node
                .properties
                .iter_mut()
                .fold(false, |changed, (_, value)| {
                    walk_expr(value, visit) | changed
                });
            let where_changed = node
                .inline_where
                .as_mut()
                .is_some_and(|value| walk_expr(value, visit));
            property_changed | where_changed
        }
        PatternElement::Edge(edge) => {
            let property_changed = edge
                .properties
                .iter_mut()
                .fold(false, |changed, (_, value)| {
                    walk_expr(value, visit) | changed
                });
            let where_changed = edge
                .inline_where
                .as_mut()
                .is_some_and(|value| walk_expr(value, visit));
            property_changed | where_changed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use selene_core::IStr;

    use crate::{
        BinaryOp, Literal, SourceSpan,
        analyze::{AnalyzedType, BindingId, ExprId},
        plan::{BindingElement, FilterPredicateKind},
    };

    fn istr(value: &str) -> IStr {
        selene_core::intern_with_admission(value)
            .expect("test string interns")
            .0
    }

    fn span() -> SourceSpan {
        SourceSpan::new(0, 1)
    }

    fn binding(raw: u32, name: &str) -> BindingDef {
        BindingDef {
            binding: BindingId::new(raw),
            name: istr(name),
            element: BindingElement::Node,
            ty: AnalyzedType::DYNAMIC,
            label_predicate: None,
            span: span(),
        }
    }

    fn variable(name: &str) -> ValueExpr {
        ValueExpr::Variable {
            name: istr(name),
            span: span(),
        }
    }

    fn binary_refs(left: &str, right: &str) -> ValueExpr {
        ValueExpr::BinaryOp {
            op: BinaryOp::Eq,
            lhs: Box::new(variable(left)),
            rhs: Box::new(variable(right)),
            span: span(),
        }
    }

    fn dynamic_project(expr: ValueExpr, refs: Vec<BindingId>) -> ProjectExpr {
        ProjectExpr {
            expr,
            expr_id: ExprId::new(0),
            ty: AnalyzedType::DYNAMIC,
            alias: None,
            binding_refs: refs,
            span: span(),
        }
    }

    #[test]
    fn walk_and_sync_filter_refreshes_binding_refs() {
        let bindings = vec![binding(0, "n"), binding(1, "m")];
        let n = bindings[0].binding;
        let m = bindings[1].binding;
        let mut predicate = FilterPredicate {
            expr: binary_refs("n", "m"),
            expr_id: ExprId::new(0),
            ty: AnalyzedType::DYNAMIC,
            binding_refs: vec![n, m],
            kind: FilterPredicateKind::Expression,
            index_consumed: false,
            span: span(),
        };

        let changed = walk_and_sync_binding_refs_filter(&mut predicate, &bindings, &mut |expr| {
            if matches!(expr, ValueExpr::BinaryOp { .. }) {
                *expr = variable("n");
                true
            } else {
                false
            }
        });

        assert!(changed);
        assert_eq!(predicate.binding_refs, vec![n]);
    }

    #[test]
    fn walk_and_sync_filter_preserves_property_equals_binding() {
        let bindings = vec![binding(0, "n")];
        let n = bindings[0].binding;
        let mut predicate = FilterPredicate {
            expr: ValueExpr::BinaryOp {
                op: BinaryOp::Add,
                lhs: Box::new(ValueExpr::Literal(Literal::Integer(1, span()))),
                rhs: Box::new(ValueExpr::Literal(Literal::Integer(2, span()))),
                span: span(),
            },
            expr_id: ExprId::new(0),
            ty: AnalyzedType::DYNAMIC,
            binding_refs: vec![n],
            kind: FilterPredicateKind::PropertyEquals {
                binding: Some(n),
                key: istr("age"),
            },
            index_consumed: false,
            span: span(),
        };

        let changed = walk_and_sync_binding_refs_filter(&mut predicate, &bindings, &mut |expr| {
            if matches!(expr, ValueExpr::BinaryOp { .. }) {
                *expr = ValueExpr::Literal(Literal::Integer(3, span()));
                true
            } else {
                false
            }
        });

        assert!(changed);
        assert_eq!(predicate.binding_refs, vec![n]);
    }

    #[test]
    fn walk_and_sync_property_init_refreshes_binding_refs() {
        let bindings = vec![binding(0, "n"), binding(1, "m")];
        let n = bindings[0].binding;
        let m = bindings[1].binding;
        let mut init = PropertyInit {
            key: istr("age"),
            value: dynamic_project(binary_refs("n", "m"), vec![n, m]),
            span: span(),
        };

        let changed = walk_and_sync_binding_refs_property_init(&mut init, &bindings, &mut |expr| {
            if matches!(expr, ValueExpr::BinaryOp { .. }) {
                *expr = variable("m");
                true
            } else {
                false
            }
        });

        assert!(changed);
        assert_eq!(init.value.binding_refs, vec![m]);
    }
}
