//! Test-harness plan snapshot summaries.

mod bounds_detail;
mod catalog_summary;
mod op_summary;
mod pattern;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    analyze::BindingId,
    plan::{Aggregate, BindingTableColumn, ExecutionPlan, PipelineOp, PlannedYieldItem, YieldKind},
};

use super::{DEFAULT_RULES, OptimizeContext, RULE_NAMES, Rule};
use catalog_summary::catalog_summary;
use op_summary::{mutation_summary, session_summary, tx_summary};
use pattern::{binding_map, binding_refs, join_tree_shape, pattern_snapshot};

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
        PipelineOp::Unwind {
            source,
            alias,
            position,
            ..
        } => {
            let payload = if let Some(position) = position {
                let kind = match position.kind {
                    crate::RowExpansionPositionKind::Ordinality => "ordinality",
                    crate::RowExpansionPositionKind::Offset => "offset",
                };
                format!(
                    "alias={}, position={}:{}, source=binding_refs=[{}]",
                    alias.as_str(),
                    kind,
                    position.alias.as_str(),
                    binding_refs(&source.binding_refs, bindings)
                )
            } else {
                format!(
                    "alias={}, source=binding_refs=[{}]",
                    alias.as_str(),
                    binding_refs(&source.binding_refs, bindings)
                )
            };
            PipelineOpSummary {
                kind: "Unwind",
                payload,
            }
        }
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
        PipelineOp::TrimOrderCarriers { projected_width } => PipelineOpSummary {
            kind: "TrimOrderCarriers",
            payload: format!("projected_width={projected_width}"),
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
        PipelineOp::CorrelatedChain(rhs) => PipelineOpSummary {
            kind: "CorrelatedChain",
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
