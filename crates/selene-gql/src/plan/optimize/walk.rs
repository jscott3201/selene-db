//! Shared optimizer walkers.

use crate::{
    IsCheckKind, PatternElement, ValueExpr,
    plan::{
        BindingTableSchema, CatalogOp, EdgeMatch, ExecutionPlan, FilterPredicate, JoinTree,
        MutationOp, PipelineOp, PlannedTypePropertyConstraint, Transformed,
        optimize::{OptimizeContext, Rule},
    },
};

/// Visit every expression-bearing IR site in one plan, excluding nested
/// subplans reached through `Union`, `Chain`, and `JoinTree::Subplan`.
pub(crate) fn walk_value_exprs(
    plan: &mut ExecutionPlan,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    let mut changed = false;
    if let Some(pattern) = &mut plan.pattern_plan {
        changed |= walk_predicates(&mut pattern.filters, visit);
        changed |= walk_join_tree_exprs(&mut pattern.join_tree, visit);
    }
    for op in &mut plan.pipeline {
        changed |= walk_pipeline_op_exprs(op, visit);
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
            PipelineOp::Filter(_)
            | PipelineOp::Project(_)
            | PipelineOp::Let(_)
            | PipelineOp::Unwind { .. }
            | PipelineOp::OrderBy(_)
            | PipelineOp::Limit { .. }
            | PipelineOp::TopK { .. }
            | PipelineOp::GroupBy { .. }
            | PipelineOp::Distinct
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
        JoinTree::Scan(_) | JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => false,
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
        JoinTree::Expand { child, .. } => recurse_join_tree_subplans(child, visit),
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
        pattern_plan: None,
        pipeline: Vec::new(),
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: Default::default(),
        next_expr_id: crate::ExprId::new(0),
    }
}

fn walk_join_tree_exprs(
    tree: &mut JoinTree,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match tree {
        JoinTree::Scan(scan) => walk_predicates(&mut scan.property_predicates, visit),
        JoinTree::Expand { child, edge, .. } => {
            let changed_child = walk_join_tree_exprs(child, visit);
            let changed_edge = walk_predicates(&mut edge.property_predicates, visit)
                | walk_predicates(&mut edge.right_property_predicates, visit);
            changed_child | changed_edge
        }
        JoinTree::HashJoin { left, right, .. } => {
            walk_join_tree_exprs(left, visit) | walk_join_tree_exprs(right, visit)
        }
        JoinTree::Outer {
            left,
            right,
            right_filters,
            ..
        } => {
            walk_join_tree_exprs(left, visit)
                | walk_join_tree_exprs(right, visit)
                | walk_predicates(right_filters, visit)
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => false,
    }
}

fn walk_pipeline_op_exprs(
    op: &mut PipelineOp,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match op {
        PipelineOp::Filter(pred) => walk_expr(&mut pred.expr, visit),
        PipelineOp::Project(items) | PipelineOp::Let(items) => {
            items.iter_mut().fold(false, |changed, item| {
                walk_expr(&mut item.expr, visit) | changed
            })
        }
        PipelineOp::Unwind { source, .. } => walk_expr(&mut source.expr, visit),
        PipelineOp::OrderBy(keys) => keys.iter_mut().fold(false, |changed, key| {
            walk_expr(&mut key.expr, visit) | changed
        }),
        PipelineOp::TopK { keys, .. } => keys.iter_mut().fold(false, |changed, key| {
            walk_expr(&mut key.expr, visit) | changed
        }),
        PipelineOp::GroupBy { keys, aggregates } => {
            let key_changed = keys.iter_mut().fold(false, |changed, key| {
                walk_expr(&mut key.expr, visit) | changed
            });
            let aggregate_changed = aggregates.iter_mut().fold(false, |changed, aggregate| {
                aggregate.args.iter_mut().fold(false, |arg_changed, arg| {
                    walk_expr(&mut arg.expr, visit) | arg_changed
                }) | changed
            });
            key_changed | aggregate_changed
        }
        PipelineOp::Call(call) => call.args.iter_mut().fold(false, |changed, arg| {
            walk_expr(&mut arg.expr, visit) | changed
        }),
        PipelineOp::Mutation(mutation) => walk_mutation_exprs(mutation, visit),
        PipelineOp::Catalog(catalog) => walk_catalog_exprs(catalog, visit),
        PipelineOp::Limit { .. }
        | PipelineOp::Distinct
        | PipelineOp::Union { .. }
        | PipelineOp::Chain(_)
        | PipelineOp::Tx(_) => false,
    }
}

fn walk_mutation_exprs(
    mutation: &mut MutationOp,
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    match mutation {
        MutationOp::InsertNode { property_inits, .. }
        | MutationOp::InsertEdge { property_inits, .. } => {
            property_inits.iter_mut().fold(false, |changed, init| {
                walk_expr(&mut init.value.expr, visit) | changed
            })
        }
        MutationOp::SetProperty { value, .. } => walk_expr(&mut value.expr, visit),
        MutationOp::SetLabel { .. }
        | MutationOp::RemoveProperty { .. }
        | MutationOp::RemoveLabel { .. }
        | MutationOp::DeleteTarget { .. } => false,
    }
}

fn walk_catalog_exprs(
    catalog: &mut CatalogOp,
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
                            walk_expr(&mut expr.expr, visit) | constraint_changed
                        }
                        PlannedTypePropertyConstraint::NotNull(_)
                        | PlannedTypePropertyConstraint::Immutable(_)
                        | PlannedTypePropertyConstraint::Unique(_)
                        | PlannedTypePropertyConstraint::Indexed(_)
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
        | CatalogOp::ShowEdgeTypes(_) => false,
    }
}

fn walk_predicates(
    predicates: &mut [FilterPredicate],
    visit: &mut impl FnMut(&mut ValueExpr) -> bool,
) -> bool {
    predicates.iter_mut().fold(false, |changed, pred| {
        walk_expr(&mut pred.expr, visit) | changed
    })
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
