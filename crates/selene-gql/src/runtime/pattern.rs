//! Pattern join-tree executor.

use std::collections::BTreeMap;

use rustc_hash::FxHashSet;
use selene_core::{IStr, NodeId, Value};

use crate::{
    AnalyzedType, BindingElement, BindingId, BindingTableColumn, BindingTableSchema,
    FilterPredicate, GqlType, HiddenBindingId, JoinTree, PatternPlan, ScanKind, SubqueryRegistry,
    analyze::ExprIdLookup,
    runtime::{Binding, BindingTable, EvalCtx, ExecutorError, TxContext},
};

use super::{
    evaluator, expand, hash_join, match_mode, outer, path_mode, questioned, scan, subplan,
    value_compare, wco,
};

/// Execute a pattern plan and produce its initial binding table.
pub fn execute_pattern(
    pattern: &PatternPlan,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let expr_ids = ExprIdLookup::default();
    let subqueries = SubqueryRegistry::default();
    let (expr_ids, subqueries) = ctx.plan_metadata().unwrap_or((&expr_ids, &subqueries));
    let eval_ctx = EvalCtx {
        tx: ctx,
        expr_ids,
        subqueries,
    };
    execute_pattern_with_seed(pattern, None, &eval_ctx)
}

/// Execute a pattern plan with an optional row seed for correlated subqueries.
pub(crate) fn execute_pattern_with_seed(
    pattern: &PatternPlan,
    seed: Option<&Binding>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let schema = schema_for_pattern(pattern);
    execute_pattern_with_seed_and_schema(pattern, seed, schema, ctx)
}

pub(crate) fn execute_pattern_with_seed_and_schema(
    pattern: &PatternPlan,
    seed: Option<&Binding>,
    schema: BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let env = WalkContext {
        pattern,
        schema: &schema,
        seed,
        ctx,
    };
    let mut rows = Vec::new();
    let mut rows_since_check = 0;
    for row in walk_join_tree(&pattern.join_tree, env)? {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        if pattern_filters_pass(pattern, &row, &schema, ctx)? {
            rows.push(row);
        }
    }
    Ok(BindingTable::new(schema, rows))
}

#[derive(Clone, Copy)]
pub(crate) struct WalkContext<'a, 'seed, 'eval, 'ctx, 'g, 'plan> {
    pub(crate) pattern: &'a PatternPlan,
    pub(crate) schema: &'a BindingTableSchema,
    pub(crate) seed: Option<&'seed Binding>,
    pub(crate) ctx: &'a EvalCtx<'eval, 'ctx, 'g, 'plan>,
}

pub(crate) fn walk_join_tree(
    tree: &JoinTree,
    env: WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    match tree {
        JoinTree::Scan(scan_node) => {
            scan::scan_pattern(scan_node, env.pattern, env.schema, env.seed, env.ctx)
        }
        JoinTree::Expand {
            child,
            edge,
            direction,
        } => expand::execute(child, edge, *direction, env),
        JoinTree::Questioned {
            child,
            edge,
            direction,
            ..
        } => questioned::execute(child, edge, *direction, env),
        JoinTree::Repeat {
            child,
            edge,
            direction,
            min,
            max,
            path_mode,
        } => super::repeat::execute(child, edge, *direction, *min, *max, *path_mode, env),
        JoinTree::PathSearch {
            selector,
            child,
            source_binding,
            final_binding,
            hop_contributors,
        } => super::path_search::execute(
            child,
            *selector,
            *source_binding,
            *final_binding,
            hop_contributors,
            env,
        ),
        JoinTree::PathModeFilter {
            path_mode,
            child,
            path_contributors,
        } => path_mode::execute(child, *path_mode, path_contributors, env),
        JoinTree::MatchModeFilter {
            match_mode,
            child,
            path_contributors,
        } => match_mode::execute(child, *match_mode, path_contributors, env),
        JoinTree::HashJoin {
            left,
            right,
            key,
            build_side,
        } => hash_join::execute(left, right, key, *build_side, env),
        JoinTree::Outer {
            left,
            right,
            key,
            right_filters,
        } => outer::execute(left, right, key, right_filters, env),
        JoinTree::WorstCaseOptimal { intersection, .. } => wco::execute_phase_a(intersection, env),
        JoinTree::Subplan(plan) => subplan::execute(plan, env.schema, env.seed, env.ctx),
        // Iterate each per-label branch and dedup the resulting `Binding`
        // rows by the scan anchor's `NodeId`. A node carrying labels A AND B
        // would otherwise appear in both branch-A and branch-B candidate
        // rows (a per-branch `UNION ALL` shape), which would change query
        // semantics observably (COUNT, LIMIT, aggregates) based on whether
        // the disjunctive-label-expansion rule fired vs the unexpanded
        // `LabelExpr::Disjunction(any(...))` filter that visits each node
        // once. The dedup at JoinTree-level restores the
        // catalog-present-vs-catalog-absent invariant: same query + same
        // data => same rows out, regardless of which optimizer rule slot
        // fired (BRIEF-155 PR #177 Codex C1).
        //
        // Dedup happens here, before downstream pipeline ops (LIMIT, ORDER
        // BY, GROUP BY, Distinct), so the union-then-dedup'd binding table
        // is what those ops see — preserving the option-b architecture
        // advantage (single union point, no per-branch pipeline fan-out).
        //
        // `scan_anchor` carries the original disjunctive `label_predicate`
        // for EXPLAIN diagnostics and IR round-trips; at execute time we
        // also use its binding / hidden-binding IDs to locate the column
        // that holds the per-row `Value::NodeRef(NodeId)` for dedup. All
        // branches inherit those IDs from `scan_anchor` (constructed by
        // `plan/optimize/rules/disjunctive_label_expansion.rs`), so all
        // branches write into the same column.
        //
        // The rule is gated to `ScanKind::Node` (see
        // `disjunctive_label_expansion::maybe_expand_scan`), so the anchor
        // column always carries `Value::NodeRef` (per
        // `runtime/scan::entity_value`). The non-`NodeRef` arm is
        // defensive and currently unreachable.
        JoinTree::DisjunctiveScan {
            branches,
            scan_anchor,
        } => {
            let anchor_index = scan_anchor
                .binding
                .and_then(|binding_id| binding_index(env.pattern, env.schema, binding_id))
                .or_else(|| {
                    scan_anchor
                        .hidden_binding
                        .and_then(|hidden_id| hidden_index(env.schema, hidden_id))
                })
                .ok_or(ExecutorError::ImplementationDefined {
                    detail: "DisjunctiveScan anchor binding missing from pattern schema",
                })?;
            let mut seen: FxHashSet<NodeId> = FxHashSet::default();
            let mut rows = Vec::new();
            for branch in branches {
                for binding in
                    scan::scan_pattern(branch, env.pattern, env.schema, env.seed, env.ctx)?
                {
                    match binding.get(anchor_index) {
                        Some(Value::NodeRef(id)) => {
                            if seen.insert(*id) {
                                rows.push(binding);
                            }
                        }
                        _ => {
                            // Defensive: rule is gated to ScanKind::Node, so
                            // this arm is unreachable. Preserve the row
                            // rather than silently drop it on the impossible
                            // variant.
                            rows.push(binding);
                        }
                    }
                }
            }
            Ok(rows)
        }
    }
}

pub(crate) fn schema_for_pattern(pattern: &PatternPlan) -> BindingTableSchema {
    let mut columns = pattern
        .bindings
        .iter()
        .filter(|binding| {
            matches!(
                binding.element,
                BindingElement::Node | BindingElement::Edge | BindingElement::Path
            )
        })
        .map(|binding| BindingTableColumn {
            name: Some(binding.name.clone()),
            hidden: None,
            ty: binding.ty.clone(),
        })
        .collect::<Vec<_>>();
    let mut hidden = BTreeMap::new();
    collect_hidden_slots(&pattern.join_tree, &mut hidden);
    columns.extend(hidden.into_iter().map(|(hidden, ty)| BindingTableColumn {
        name: None,
        hidden: Some(hidden),
        ty,
    }));
    BindingTableSchema { columns }
}

fn collect_hidden_slots(tree: &JoinTree, slots: &mut BTreeMap<HiddenBindingId, AnalyzedType>) {
    match tree {
        JoinTree::Scan(scan) => {
            insert_hidden(slots, scan.hidden_binding, scan.kind);
        }
        JoinTree::Expand { child, edge, .. } => {
            collect_hidden_slots(child, slots);
            insert_hidden(slots, edge.left_hidden_binding, ScanKind::Node);
            insert_hidden(slots, edge.hidden_binding, ScanKind::Edge);
            insert_hidden(slots, edge.right_hidden_binding, ScanKind::Node);
        }
        JoinTree::Questioned { child, edge, .. } => {
            collect_hidden_slots(child, slots);
            insert_hidden(slots, edge.left_hidden_binding, ScanKind::Node);
            insert_hidden(slots, edge.hidden_binding, ScanKind::Edge);
            insert_hidden(slots, edge.right_hidden_binding, ScanKind::Node);
        }
        JoinTree::Repeat { child, edge, .. } => {
            collect_hidden_slots(child, slots);
            insert_hidden(slots, edge.left_hidden_binding, ScanKind::Node);
            insert_hidden_type(
                slots,
                edge.group_hidden_binding,
                AnalyzedType::Resolved(GqlType::List(Box::new(GqlType::EdgeRef))),
            );
            insert_hidden(slots, edge.final_hidden_binding, ScanKind::Node);
        }
        JoinTree::PathSearch { child, .. }
        | JoinTree::PathModeFilter { child, .. }
        | JoinTree::MatchModeFilter { child, .. } => {
            collect_hidden_slots(child, slots);
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            collect_hidden_slots(left, slots);
            collect_hidden_slots(right, slots);
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for tree in intersection {
                collect_hidden_slots(tree, slots);
            }
        }
        JoinTree::Subplan(_) => {}
        JoinTree::DisjunctiveScan { branches, .. } => {
            for branch in branches {
                insert_hidden(slots, branch.hidden_binding, branch.kind);
            }
        }
    }
}

fn insert_hidden(
    slots: &mut BTreeMap<HiddenBindingId, AnalyzedType>,
    hidden: Option<HiddenBindingId>,
    kind: ScanKind,
) {
    let Some(hidden) = hidden else {
        return;
    };
    slots.entry(hidden).or_insert_with(|| {
        AnalyzedType::Resolved(match kind {
            ScanKind::Node => GqlType::NodeRef,
            ScanKind::Edge => GqlType::EdgeRef,
        })
    });
}

fn insert_hidden_type(
    slots: &mut BTreeMap<HiddenBindingId, AnalyzedType>,
    hidden: Option<HiddenBindingId>,
    ty: AnalyzedType,
) {
    let Some(hidden) = hidden else {
        return;
    };
    slots.entry(hidden).or_insert(ty);
}

pub(crate) fn binding_index(
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    binding_id: BindingId,
) -> Option<usize> {
    let binding = pattern
        .bindings
        .iter()
        .find(|candidate| candidate.binding == binding_id)?;
    column_index(schema, binding.name.clone())
}

pub(crate) fn column_index(schema: &BindingTableSchema, name: IStr) -> Option<usize> {
    schema
        .columns
        .iter()
        .position(|column| column.name == Some(name.clone()))
}

pub(crate) fn hidden_index(schema: &BindingTableSchema, hidden: HiddenBindingId) -> Option<usize> {
    schema
        .columns
        .iter()
        .position(|column| column.hidden == Some(hidden))
}

pub(crate) fn set_binding_value(
    values: &mut [Value],
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    binding_id: Option<BindingId>,
    value: Value,
) -> Result<bool, ExecutorError> {
    let Some(binding_id) = binding_id else {
        return Ok(true);
    };
    let Some(index) = binding_index(pattern, schema, binding_id) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "binding column missing from pattern schema",
        });
    };
    if !matches!(values[index], Value::Null)
        && !value_compare::equal_non_null(&values[index], &value)
    {
        return Ok(false);
    }
    values[index] = value;
    Ok(true)
}

pub(crate) fn set_hidden_value(
    values: &mut [Value],
    schema: &BindingTableSchema,
    hidden: Option<HiddenBindingId>,
    value: Value,
) -> Result<bool, ExecutorError> {
    let Some(hidden) = hidden else {
        return Ok(true);
    };
    let Some(index) = hidden_index(schema, hidden) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "hidden binding column missing from pattern schema",
        });
    };
    if !matches!(values[index], Value::Null)
        && !value_compare::equal_non_null(&values[index], &value)
    {
        return Ok(false);
    }
    values[index] = value;
    Ok(true)
}

pub(crate) fn merge_rows(left: &Binding, right: &Binding, schema: &BindingTableSchema) -> Binding {
    let mut values = Vec::with_capacity(schema.columns.len());
    for index in 0..schema.columns.len() {
        let left_value = left.get(index).cloned().unwrap_or(Value::Null);
        // Null is the row-local unbound sentinel; prefer the bound side.
        if matches!(left_value, Value::Null) {
            values.push(right.get(index).cloned().unwrap_or(Value::Null));
        } else {
            values.push(left_value);
        }
    }
    Binding::new(values)
}

pub(crate) fn key_values(
    row: &Binding,
    schema: &BindingTableSchema,
    key: &[IStr],
) -> Result<Option<Vec<Value>>, ExecutorError> {
    let mut values = Vec::with_capacity(key.len());
    for name in key {
        let Some(index) = column_index(schema, name.clone()) else {
            return Err(ExecutorError::ImplementationDefined {
                detail: "join key column missing from pattern schema",
            });
        };
        let value = row.get(index).cloned().unwrap_or(Value::Null);
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        values.push(value);
    }
    Ok(Some(values))
}

pub(crate) fn key_values_equal(lhs: &[Value], rhs: &[Value]) -> bool {
    lhs.len() == rhs.len()
        && lhs
            .iter()
            .zip(rhs)
            .all(|(lhs, rhs)| value_compare::equal_non_null(lhs, rhs))
}

pub(crate) fn rows_match_on_key(
    left: &Binding,
    right: &Binding,
    schema: &BindingTableSchema,
    key: &[IStr],
) -> Result<bool, ExecutorError> {
    let Some(left_key) = key_values(left, schema, key)? else {
        return Ok(false);
    };
    let Some(right_key) = key_values(right, schema, key)? else {
        return Ok(false);
    };
    Ok(key_values_equal(&left_key, &right_key))
}

pub(crate) fn project_row_to_schema(
    row: &Binding,
    source_schema: &BindingTableSchema,
    target_schema: &BindingTableSchema,
    seed: Option<&Binding>,
) -> Binding {
    let mut values = seed
        .map(|row| row.values().to_vec())
        .unwrap_or_else(|| vec![Value::Null; target_schema.columns.len()]);
    values.resize(target_schema.columns.len(), Value::Null);
    for (target_index, target_column) in target_schema.columns.iter().enumerate() {
        let Some(ref name) = target_column.name else {
            continue;
        };
        let Some(source_index) = column_index(source_schema, name.clone()) else {
            continue;
        };
        values[target_index] = row.get(source_index).cloned().unwrap_or(Value::Null);
    }
    Binding::new(values)
}

fn pattern_filters_pass(
    pattern: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    filter_predicates_pass(&pattern.filters, pattern, row, schema, ctx)
}

pub(crate) fn filter_predicates_pass(
    predicates: &[FilterPredicate],
    pattern: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    for predicate in predicates {
        if !filter_predicate_passes(predicate, pattern, row, schema, ctx)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn filter_predicate_passes(
    predicate: &FilterPredicate,
    pattern: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    if predicate.index_consumed {
        return Ok(true);
    }
    match predicate.kind {
        crate::FilterPredicateKind::Expression => {
            let value = evaluator::evaluate(&predicate.expr, row, schema, ctx)?;
            Ok(matches!(value, Value::Bool(true)))
        }
        crate::FilterPredicateKind::PropertyEquals { .. } => {
            scan::predicate_passes(predicate, pattern, row, schema, &Value::Null, ctx)
        }
    }
}
