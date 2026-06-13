//! Test-harness plan snapshot summaries.

mod bounds_detail;
mod catalog_summary;
mod op_summary;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    analyze::BindingId,
    plan::{
        Aggregate, BindingDef, BindingTableColumn, EdgeMatch, ExecutionPlan, FilterPredicate,
        JoinTree, NodeOrEdgeScan, OrderAccess, OrderKey, PipelineOp, PlannedYieldItem,
        RepeatEdgeMatch, ScanAccess, ScanKind, YieldKind,
    },
};

use super::{DEFAULT_RULES, OptimizeContext, RULE_NAMES, Rule};
use bounds_detail::bounds_detail_for_access;
use catalog_summary::catalog_summary;
use op_summary::{mutation_summary, session_summary, tx_summary};

/// Optimize a plan and return a deterministic summary for snapshot tests.
#[must_use]
pub fn optimize_summary(plan: ExecutionPlan, ctx: &OptimizeContext<'_>) -> PlanSnapshot {
    let (plan, fired_rules) = optimize_recording(plan, DEFAULT_RULES, ctx);
    PlanSnapshot::from_plan(&plan, fired_rules)
}

/// Stable, compact optimized-plan snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PlanSnapshot {
    /// Optimized pipeline operations in order.
    pub pipeline_ops: Vec<PipelineOpSummary>,
    /// Leading pattern summary, when the plan has a leading pattern phase.
    pub pattern: Option<PatternSnapshot>,
    /// Final output columns.
    pub output_columns: Vec<String>,
    /// Optimizer rules that reported a change at least once.
    pub fired_rules: Vec<&'static str>,
    /// Pipeline-op high-water mark after optimization. Captured so a test can
    /// assert the recording driver agrees with the production
    /// `optimize_with_rules` (PLAN-20). Deliberately NOT rendered by
    /// [`fmt::Display`] so the golden `.snap` corpus is unaffected.
    pub next_pipeline_op_id: u32,
}

/// Stable summary of one pipeline operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PipelineOpSummary {
    /// Pipeline operation variant name.
    pub kind: &'static str,
    /// Compact, deterministic payload summary.
    pub payload: String,
}

/// Stable summary of a leading pattern plan.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PatternSnapshot {
    /// Number of pattern bindings.
    pub binding_count: usize,
    /// Pattern binding names, sorted for deterministic display.
    pub binding_names: Vec<String>,
    /// Compact join-tree shape.
    pub join_tree_shape: String,
    /// Number of predicates left at the pattern boundary.
    pub pattern_filter_count: usize,
    /// Scan and expansion access summaries.
    pub scans: Vec<ScanSnapshot>,
    /// Order access hints visible in downstream `OrderBy` or `TopK` keys.
    pub order_access: Vec<Option<String>>,
}

/// Stable summary of a scan-like access site.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ScanSnapshot {
    /// Binding name, or an anonymous placeholder.
    pub binding: String,
    /// Scan element kind.
    pub kind: &'static str,
    /// Access-path tag.
    pub access: &'static str,
    /// Predicates still evaluated after access-path selection.
    pub residual_predicates: usize,
    /// Predicates marked as consumed by an index-aware rule.
    pub consumed_predicates: usize,
    /// Bounds-detail string for the three indexed-scan access paths
    /// (`TypedIndexRange` / `BitmapUnion` / `CompositeLookup`), or `None` for
    /// access variants that do not carry probe keys (`Linear`, `LabelIndex`).
    /// Renders literals as `KIND value` and parameter slots as `$name`; see
    /// `bounds_detail` for the canonical format. Additive field — existing
    /// `#[non_exhaustive]` callers continue to compile by ignoring it.
    pub bounds_detail: Option<String>,
}

impl PlanSnapshot {
    fn from_plan(plan: &ExecutionPlan, fired_rules: Vec<&'static str>) -> Self {
        let bindings = plan
            .pattern_plan
            .as_ref()
            .map(|pattern| binding_map(&pattern.bindings))
            .unwrap_or_default();
        Self {
            pipeline_ops: plan
                .pipeline
                .iter()
                .map(|op| pipeline_summary(op, &bindings))
                .collect(),
            pattern: plan
                .pattern_plan
                .as_ref()
                .map(|pattern| pattern_snapshot(pattern, &plan.pipeline)),
            output_columns: output_columns(&plan.output_schema.columns),
            fired_rules,
            next_pipeline_op_id: plan.next_pipeline_op_id.get(),
        }
    }

    fn compact(&self) -> String {
        let pipeline = self
            .pipeline_ops
            .iter()
            .map(|op| op.kind)
            .collect::<Vec<_>>()
            .join(",");
        let pattern = self
            .pattern
            .as_ref()
            .map(|pattern| pattern.join_tree_shape.as_str())
            .unwrap_or("none");
        format!(
            "pipeline=[{}], output=[{}], pattern={}",
            pipeline,
            self.output_columns.join(","),
            pattern
        )
    }
}

impl fmt::Display for PlanSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "fired_rules: {}", display_list(&self.fired_rules))?;
        writeln!(f, "output: [{}]", self.output_columns.join(", "))?;
        if let Some(pattern) = &self.pattern {
            writeln!(f, "pattern:")?;
            writeln!(
                f,
                "  bindings: {} [{}]",
                pattern.binding_count,
                pattern.binding_names.join(", ")
            )?;
            writeln!(f, "  join_tree: {}", pattern.join_tree_shape)?;
            writeln!(f, "  pattern_filters: {}", pattern.pattern_filter_count)?;
            writeln!(f, "  scans:")?;
            if pattern.scans.is_empty() {
                writeln!(f, "    - none")?;
            } else {
                for scan in &pattern.scans {
                    let bounds_suffix = scan
                        .bounds_detail
                        .as_deref()
                        .map(|detail| format!(" [bounds={detail}]"))
                        .unwrap_or_default();
                    writeln!(
                        f,
                        "    - {} ({}): {}{} residual={} consumed={}",
                        scan.binding,
                        scan.kind,
                        scan.access,
                        bounds_suffix,
                        scan.residual_predicates,
                        scan.consumed_predicates
                    )?;
                }
            }
            writeln!(
                f,
                "  order_access: [{}]",
                pattern
                    .order_access
                    .iter()
                    .map(|access| access.as_deref().unwrap_or("none"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        } else {
            writeln!(f, "pattern: none")?;
        }
        writeln!(f, "pipeline:")?;
        if self.pipeline_ops.is_empty() {
            writeln!(f, "  - none")?;
        } else {
            for op in &self.pipeline_ops {
                if op.payload.is_empty() {
                    writeln!(f, "  - {}", op.kind)?;
                } else {
                    writeln!(f, "  - {}({})", op.kind, op.payload)?;
                }
            }
        }
        Ok(())
    }
}

fn optimize_recording(
    mut plan: ExecutionPlan,
    rules: &[&'static dyn Rule],
    ctx: &OptimizeContext<'_>,
) -> (ExecutionPlan, Vec<&'static str>) {
    let mut fired = BTreeSet::new();
    for _ in 0..ctx.impl_defined_caps.max_optimizer_iterations {
        let mut changed_any = false;
        for rule in rules {
            let transformed = rule.rewrite(plan, ctx);
            plan = transformed.plan;
            if transformed.changed {
                fired.insert(rule.name());
                changed_any = true;
            }
        }
        if !changed_any {
            break;
        }
    }
    // Why (PLAN-20): mirror the production `optimize_with_rules` post-loop
    // step. Without this, the recording driver leaves `next_pipeline_op_id` at
    // its lowering-time value while production refreshes it from the final
    // pipeline length, so the corpus + idempotence snapshots would pin the
    // test driver instead of the shipped optimizer. Parity is asserted in the
    // `recording_driver_matches_production_pipeline_op_high_water` test below.
    plan.refresh_pipeline_op_high_water();
    let mut fired_rules = fired.into_iter().collect::<Vec<_>>();
    fired_rules.sort_unstable_by_key(|name| {
        RULE_NAMES
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or(usize::MAX)
    });
    (plan, fired_rules)
}

fn pattern_snapshot(pattern: &crate::PatternPlan, pipeline: &[PipelineOp]) -> PatternSnapshot {
    let bindings = binding_map(&pattern.bindings);
    let mut binding_names = bindings.values().cloned().collect::<Vec<_>>();
    binding_names.sort();
    binding_names.dedup();
    let mut scans = Vec::new();
    collect_scans(&pattern.join_tree, &bindings, &mut scans);
    PatternSnapshot {
        binding_count: pattern.bindings.len(),
        binding_names,
        join_tree_shape: join_tree_shape(&pattern.join_tree, &bindings),
        pattern_filter_count: pattern.filters.len(),
        scans,
        order_access: collect_order_access(pipeline),
    }
}

fn pipeline_summary(op: &PipelineOp, bindings: &BTreeMap<BindingId, String>) -> PipelineOpSummary {
    match op {
        PipelineOp::Filter(pred) => PipelineOpSummary {
            kind: "Filter",
            payload: format!(
                "binding_refs=[{}]",
                binding_refs(&pred.binding_refs, bindings)
            ),
        },
        PipelineOp::Project(items) => PipelineOpSummary {
            kind: "Project",
            payload: format!(
                "columns=[{}]",
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| item
                        .alias
                        .clone()
                        .map(|alias| alias.as_str().to_owned())
                        .unwrap_or_else(|| format!("expr{index}")))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        },
        PipelineOp::Let(items) => PipelineOpSummary {
            kind: "Let",
            payload: format!(
                "bindings=[{}]",
                items
                    .iter()
                    .filter_map(|item| item.alias.clone().map(|alias| alias.as_str().to_owned()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        },
        PipelineOp::Unwind { source, alias, .. } => PipelineOpSummary {
            kind: "Unwind",
            payload: format!(
                "alias={}, source=binding_refs=[{}]",
                alias.as_str(),
                binding_refs(&source.binding_refs, bindings)
            ),
        },
        PipelineOp::OrderBy(keys) => PipelineOpSummary {
            kind: "OrderBy",
            payload: format!("keys={}", keys.len()),
        },
        PipelineOp::Limit { offset, count } => PipelineOpSummary {
            kind: "Limit",
            payload: format!("offset={offset:?}, count={count:?}"),
        },
        PipelineOp::TopK {
            keys,
            offset,
            count,
        } => PipelineOpSummary {
            kind: "TopK",
            payload: format!("keys={}, offset={offset:?}, count={count:?}", keys.len()),
        },
        PipelineOp::GroupBy { keys, aggregates } => PipelineOpSummary {
            kind: "GroupBy",
            payload: format!(
                "keys={}, aggs=[{}]",
                keys.len(),
                aggregates
                    .iter()
                    .map(aggregate_summary)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        },
        PipelineOp::Distinct => PipelineOpSummary {
            kind: "Distinct",
            payload: String::new(),
        },
        PipelineOp::Union { op, rhs } => PipelineOpSummary {
            kind: "Union",
            payload: format!(
                "op={op:?}, rhs={}",
                PlanSnapshot::from_plan(rhs, Vec::new()).compact()
            ),
        },
        PipelineOp::Chain(rhs) => PipelineOpSummary {
            kind: "Chain",
            payload: format!("rhs={}", PlanSnapshot::from_plan(rhs, Vec::new()).compact()),
        },
        PipelineOp::Match(pattern) => PipelineOpSummary {
            kind: "Match",
            payload: join_tree_shape(&pattern.join_tree, bindings),
        },
        PipelineOp::OptionalMatch(pattern) => PipelineOpSummary {
            kind: "OptionalMatch",
            payload: join_tree_shape(&pattern.join_tree, bindings),
        },
        PipelineOp::Call(call) => PipelineOpSummary {
            kind: "Call",
            payload: format!(
                "name={}, args={}, yield=[{}]",
                call.procedure
                    .iter()
                    .map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
                call.args.len(),
                call.yield_cols
                    .iter()
                    .map(yield_summary)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        },
        PipelineOp::CallSubquery(call) => PipelineOpSummary {
            kind: "CallSubquery",
            payload: format!(
                "yield=[{}], body={}",
                call.yield_items
                    .iter()
                    .map(|item| format!("{}=>{}", item.source.as_str(), item.output.as_str()))
                    .collect::<Vec<_>>()
                    .join(","),
                PlanSnapshot::from_plan(&call.body, Vec::new()).compact()
            ),
        },
        PipelineOp::Mutation(mutation) => PipelineOpSummary {
            kind: "Mutation",
            payload: mutation_summary(mutation),
        },
        PipelineOp::Catalog(catalog) => PipelineOpSummary {
            kind: "Catalog",
            payload: catalog_summary(catalog),
        },
        PipelineOp::ExplainPlan { inner, .. } => PipelineOpSummary {
            kind: "ExplainPlan",
            payload: format!(
                "inner={}",
                PlanSnapshot::from_plan(inner, Vec::new()).compact()
            ),
        },
        PipelineOp::Tx(tx) => PipelineOpSummary {
            kind: "Tx",
            payload: tx_summary(tx),
        },
        PipelineOp::Session(session) => PipelineOpSummary {
            kind: "Session",
            payload: session_summary(session),
        },
    }
}

fn output_columns(columns: &[BindingTableColumn]) -> Vec<String> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            column
                .name
                .clone()
                .map(|name| name.as_str().to_owned())
                .unwrap_or_else(|| format!("expr{index}"))
        })
        .collect()
}

fn collect_order_access(pipeline: &[PipelineOp]) -> Vec<Option<String>> {
    let mut access = Vec::new();
    for op in pipeline {
        match op {
            PipelineOp::OrderBy(keys) => access.extend(keys.iter().map(order_access)),
            PipelineOp::TopK { keys, .. } => access.extend(keys.iter().map(order_access)),
            PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
                access.extend(collect_order_access(&rhs.pipeline));
            }
            PipelineOp::CallSubquery(call) => {
                access.extend(collect_order_access(&call.body.pipeline));
            }
            PipelineOp::ExplainPlan { inner, .. } => {
                access.extend(collect_order_access(&inner.pipeline));
            }
            PipelineOp::Filter(_)
            | PipelineOp::Project(_)
            | PipelineOp::Let(_)
            | PipelineOp::Unwind { .. }
            | PipelineOp::Limit { .. }
            | PipelineOp::GroupBy { .. }
            | PipelineOp::Distinct
            | PipelineOp::Match(_)
            | PipelineOp::OptionalMatch(_)
            | PipelineOp::Call(_)
            | PipelineOp::Mutation(_)
            | PipelineOp::Catalog(_)
            | PipelineOp::Tx(_)
            | PipelineOp::Session(_) => {}
        }
    }
    access
}

fn collect_scans(
    tree: &JoinTree,
    bindings: &BTreeMap<BindingId, String>,
    scans: &mut Vec<ScanSnapshot>,
) {
    match tree {
        JoinTree::Scan(scan) => scans.push(scan_snapshot(scan, bindings)),
        JoinTree::Expand { child, edge, .. } => {
            collect_scans(child, bindings, scans);
            scans.push(edge_snapshot(edge, bindings));
            if !edge.right_property_predicates.is_empty() {
                scans.push(ScanSnapshot {
                    binding: binding_name(edge.right_binding, bindings, "<anonymous-node>"),
                    kind: "Node",
                    access: "Linear",
                    residual_predicates: edge.right_property_predicates.len(),
                    consumed_predicates: consumed_count(&edge.right_property_predicates),
                    bounds_detail: None,
                });
            }
        }
        JoinTree::Questioned { child, edge, .. } => {
            collect_scans(child, bindings, scans);
            scans.push(edge_snapshot(edge, bindings));
            if !edge.right_property_predicates.is_empty() {
                scans.push(ScanSnapshot {
                    binding: binding_name(edge.right_binding, bindings, "<anonymous-node>"),
                    kind: "Node",
                    access: "Linear",
                    residual_predicates: edge.right_property_predicates.len(),
                    consumed_predicates: consumed_count(&edge.right_property_predicates),
                    bounds_detail: None,
                });
            }
        }
        JoinTree::Repeat { child, edge, .. } => {
            collect_scans(child, bindings, scans);
            scans.push(repeat_edge_snapshot(edge, bindings));
            if !edge.final_property_predicates.is_empty() {
                scans.push(ScanSnapshot {
                    binding: binding_name(edge.final_binding, bindings, "<anonymous-node>"),
                    kind: "Node",
                    access: "Linear",
                    residual_predicates: edge.final_property_predicates.len(),
                    consumed_predicates: consumed_count(&edge.final_property_predicates),
                    bounds_detail: None,
                });
            }
        }
        JoinTree::PathSearch { child, .. }
        | JoinTree::PathModeFilter { child, .. }
        | JoinTree::MatchModeFilter { child, .. } => {
            collect_scans(child, bindings, scans);
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            collect_scans(left, bindings, scans);
            collect_scans(right, bindings, scans);
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for child in intersection {
                collect_scans(child, bindings, scans);
            }
        }
        JoinTree::Subplan(plan) => {
            if let Some(pattern) = &plan.pattern_plan {
                collect_scans(&pattern.join_tree, &binding_map(&pattern.bindings), scans);
            }
        }
        JoinTree::DisjunctiveScan { branches, .. } => {
            // Render one ScanSnapshot per branch so EXPLAIN exposes each
            // branch's independently-selected ScanAccess (acceptance bar
            // #1; Q5 transparent rendering).
            for branch in branches {
                scans.push(scan_snapshot(branch, bindings));
            }
        }
    }
}

fn scan_snapshot(scan: &NodeOrEdgeScan, bindings: &BTreeMap<BindingId, String>) -> ScanSnapshot {
    ScanSnapshot {
        binding: binding_name(scan.binding, bindings, "<anonymous>"),
        kind: scan_kind(scan.kind),
        access: scan_access(&scan.access),
        residual_predicates: scan.property_predicates.len(),
        consumed_predicates: consumed_count(&scan.property_predicates),
        bounds_detail: bounds_detail_for_access(&scan.access),
    }
}

fn edge_snapshot(edge: &EdgeMatch, bindings: &BTreeMap<BindingId, String>) -> ScanSnapshot {
    ScanSnapshot {
        binding: binding_name(edge.binding, bindings, "<anonymous-edge>"),
        kind: "Edge",
        access: scan_access(&edge.access),
        residual_predicates: edge.property_predicates.len(),
        consumed_predicates: consumed_count(&edge.property_predicates),
        bounds_detail: bounds_detail_for_access(&edge.access),
    }
}

fn repeat_edge_snapshot(
    edge: &RepeatEdgeMatch,
    bindings: &BTreeMap<BindingId, String>,
) -> ScanSnapshot {
    ScanSnapshot {
        binding: binding_name(edge.group_binding, bindings, "<anonymous-repeat-edge>"),
        kind: "Edge",
        access: scan_access(&edge.access),
        residual_predicates: edge.property_predicates.len() + edge.inline_predicates.len(),
        consumed_predicates: consumed_count(&edge.property_predicates)
            + consumed_count(&edge.inline_predicates),
        bounds_detail: bounds_detail_for_access(&edge.access),
    }
}

fn join_tree_shape(tree: &JoinTree, bindings: &BTreeMap<BindingId, String>) -> String {
    match tree {
        JoinTree::Scan(scan) => format!("Scan({})", binding_name(scan.binding, bindings, "_")),
        JoinTree::Expand { child, edge, .. } => format!(
            "{}->Expand({}->{})",
            join_tree_shape(child, bindings),
            binding_name(edge.binding, bindings, "_"),
            binding_name(edge.right_binding, bindings, "_")
        ),
        JoinTree::Questioned { child, edge, .. } => format!(
            "{}->Questioned({}->{})",
            join_tree_shape(child, bindings),
            binding_name(edge.binding, bindings, "_"),
            binding_name(edge.right_binding, bindings, "_")
        ),
        JoinTree::Repeat {
            child,
            edge,
            min,
            max,
            ..
        } => format!(
            "{}->Repeat({}->{};{}..{})",
            join_tree_shape(child, bindings),
            binding_name(edge.group_binding, bindings, "_"),
            binding_name(edge.final_binding, bindings, "_"),
            min,
            max.map_or_else(|| "*".to_owned(), |max| max.to_string())
        ),
        JoinTree::PathSearch {
            selector, child, ..
        } => format!("{selector:?}({})", join_tree_shape(child, bindings)),
        JoinTree::PathModeFilter {
            path_mode, child, ..
        } => format!("{path_mode:?}({})", join_tree_shape(child, bindings)),
        JoinTree::MatchModeFilter {
            match_mode, child, ..
        } => format!("{match_mode:?}({})", join_tree_shape(child, bindings)),
        JoinTree::HashJoin { left, right, .. } => format!(
            "HashJoin({}, {})",
            join_tree_shape(left, bindings),
            join_tree_shape(right, bindings)
        ),
        JoinTree::Outer { left, right, .. } => format!(
            "Outer({}, {})",
            join_tree_shape(left, bindings),
            join_tree_shape(right, bindings)
        ),
        JoinTree::WorstCaseOptimal {
            intersection,
            node_id_ordering,
        } => format!(
            "WCO(intersection={}, orderings={})",
            intersection.len(),
            node_id_ordering.len()
        ),
        JoinTree::Subplan(plan) => {
            format!(
                "Subplan({})",
                PlanSnapshot::from_plan(plan, Vec::new()).compact()
            )
        }
        JoinTree::DisjunctiveScan { branches, .. } => {
            format!("Disjunctive({} branches)", branches.len())
        }
    }
}

fn binding_map(bindings: &[BindingDef]) -> BTreeMap<BindingId, String> {
    bindings
        .iter()
        .map(|binding| (binding.binding, binding.name.as_str().to_owned()))
        .collect()
}

fn binding_name(
    binding: Option<BindingId>,
    bindings: &BTreeMap<BindingId, String>,
    anonymous: &str,
) -> String {
    binding
        .and_then(|binding| bindings.get(&binding).cloned())
        .unwrap_or_else(|| anonymous.to_owned())
}

fn binding_refs(refs: &[BindingId], bindings: &BTreeMap<BindingId, String>) -> String {
    refs.iter()
        .map(|binding| {
            bindings
                .get(binding)
                .cloned()
                .unwrap_or_else(|| format!("#{}", binding.get()))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn consumed_count(predicates: &[FilterPredicate]) -> usize {
    predicates
        .iter()
        .filter(|predicate| predicate.index_consumed)
        .count()
}

fn scan_kind(kind: ScanKind) -> &'static str {
    match kind {
        ScanKind::Node => "Node",
        ScanKind::Edge => "Edge",
    }
}

fn scan_access(access: &ScanAccess) -> &'static str {
    match access {
        ScanAccess::Linear => "Linear",
        ScanAccess::LabelIndex { .. } => "LabelIndex",
        ScanAccess::TypedIndexRange { .. } => "TypedIndexRange",
        ScanAccess::BitmapUnion { .. } => "BitmapUnion",
        ScanAccess::CompositeLookup { .. } => "CompositeLookup",
    }
}

fn order_access(key: &OrderKey) -> Option<String> {
    key.access.as_ref().map(|access| match access {
        OrderAccess::TypedIndex { direction, .. } => format!("TypedIndex({direction:?})"),
    })
}

fn aggregate_summary(aggregate: &Aggregate) -> String {
    if aggregate.star {
        format!("{}(*)", aggregate.function.as_str())
    } else {
        let distinct = if aggregate.distinct { " distinct" } else { "" };
        format!(
            "{}({} args{distinct})",
            aggregate.function.as_str(),
            aggregate.args.len()
        )
    }
}

fn yield_summary(item: &PlannedYieldItem) -> String {
    let column = match &item.column {
        YieldKind::Star => "*".to_owned(),
        YieldKind::Named(name) => name.as_str().to_owned(),
    };
    item.alias
        .clone()
        .map(|alias| format!("{column} as {}", alias.as_str()))
        .unwrap_or(column)
}

fn display_list(values: &[&str]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}
