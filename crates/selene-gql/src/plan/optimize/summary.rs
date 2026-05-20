//! Test-harness plan snapshot summaries.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    LabelExpr,
    analyze::BindingId,
    plan::{
        Aggregate, BindingDef, BindingTableColumn, CatalogOp, EdgeMatch, ExecutionPlan,
        FilterPredicate, JoinTree, MutationOp, NodeOrEdgeScan, OrderAccess, OrderKey, PipelineOp,
        PlannedYieldItem, ScanAccess, ScanKind, TxOp, YieldKind,
    },
};

use super::{DEFAULT_RULES, OptimizeContext, RULE_NAMES, Rule};

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
                    writeln!(
                        f,
                        "    - {} ({}): {} residual={} consumed={}",
                        scan.binding,
                        scan.kind,
                        scan.access,
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
                    .filter_map(|item| item.alias.map(|alias| alias.as_str().to_owned()))
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
    }
}

fn output_columns(columns: &[BindingTableColumn]) -> Vec<String> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            column
                .name
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
            | PipelineOp::Call(_)
            | PipelineOp::Mutation(_)
            | PipelineOp::Catalog(_)
            | PipelineOp::Tx(_) => {}
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
                });
            }
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
    }
}

fn scan_snapshot(scan: &NodeOrEdgeScan, bindings: &BTreeMap<BindingId, String>) -> ScanSnapshot {
    ScanSnapshot {
        binding: binding_name(scan.binding, bindings, "<anonymous>"),
        kind: scan_kind(scan.kind),
        access: scan_access(&scan.access),
        residual_predicates: scan.property_predicates.len(),
        consumed_predicates: consumed_count(&scan.property_predicates),
    }
}

fn edge_snapshot(edge: &EdgeMatch, bindings: &BTreeMap<BindingId, String>) -> ScanSnapshot {
    ScanSnapshot {
        binding: binding_name(edge.binding, bindings, "<anonymous-edge>"),
        kind: "Edge",
        access: scan_access(&edge.access),
        residual_predicates: edge.property_predicates.len(),
        consumed_predicates: consumed_count(&edge.property_predicates),
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
    let column = match item.column {
        YieldKind::Star => "*".to_owned(),
        YieldKind::Named(name) => name.as_str().to_owned(),
    };
    item.alias
        .map(|alias| format!("{column} as {}", alias.as_str()))
        .unwrap_or(column)
}

fn mutation_summary(mutation: &MutationOp) -> String {
    match mutation {
        MutationOp::InsertNode {
            label_expr,
            property_inits,
            ..
        } => format!(
            "op=InsertNode(label={}, props={})",
            label_expr_summary(label_expr.as_ref()),
            property_inits.len()
        ),
        MutationOp::InsertEdge {
            label_expr,
            property_inits,
            ..
        } => format!(
            "op=InsertEdge(label={}, props={})",
            label_expr_summary(label_expr.as_ref()),
            property_inits.len()
        ),
        MutationOp::SetProperty { key, .. } => format!("op=SetProperty(key={})", key.as_str()),
        MutationOp::SetLabel { label, .. } => format!("op=SetLabel(label={})", label.as_str()),
        MutationOp::RemoveProperty { key, .. } => {
            format!("op=RemoveProperty(key={})", key.as_str())
        }
        MutationOp::RemoveLabel { label, .. } => {
            format!("op=RemoveLabel(label={})", label.as_str())
        }
        MutationOp::DeleteTarget { mode, .. } => format!("op=DeleteTarget(mode={mode:?})"),
    }
}

fn catalog_summary(catalog: &CatalogOp) -> String {
    match catalog {
        CatalogOp::CreateGraph { name, .. } => format!("op=CreateGraph(name={})", name.as_str()),
        CatalogOp::DropGraph { name, .. } => format!("op=DropGraph(name={})", name.as_str()),
        CatalogOp::CreateNodeType {
            label, properties, ..
        } => format!(
            "op=CreateNodeType(label={}, props={})",
            label.as_str(),
            properties.len()
        ),
        CatalogOp::CreateEdgeType {
            label, properties, ..
        } => format!(
            "op=CreateEdgeType(label={}, props={})",
            label.as_str(),
            properties.len()
        ),
        CatalogOp::DropNodeType { label, .. } => {
            format!("op=DropNodeType(label={})", label.as_str())
        }
        CatalogOp::DropEdgeType { label, .. } => {
            format!("op=DropEdgeType(label={})", label.as_str())
        }
        CatalogOp::ShowNodeTypes(_) => "op=ShowNodeTypes".to_owned(),
        CatalogOp::ShowEdgeTypes(_) => "op=ShowEdgeTypes".to_owned(),
        CatalogOp::ShowIndexes(_) => "op=ShowIndexes".to_owned(),
        CatalogOp::ShowProcedures(_) => "op=ShowProcedures".to_owned(),
    }
}

fn tx_summary(tx: &TxOp) -> String {
    match tx {
        TxOp::Start { .. } => "op=Start".to_owned(),
        TxOp::Commit { .. } => "op=Commit".to_owned(),
        TxOp::Rollback { .. } => "op=Rollback".to_owned(),
    }
}

fn label_expr_summary(label: Option<&LabelExpr>) -> String {
    match label {
        Some(LabelExpr::Single(label)) => label.as_str().to_owned(),
        Some(LabelExpr::Conjunction(_)) => "conjunction".to_owned(),
        Some(LabelExpr::Disjunction(_)) => "disjunction".to_owned(),
        Some(LabelExpr::Negation(_)) => "negation".to_owned(),
        Some(LabelExpr::Wildcard) => "*".to_owned(),
        None => "none".to_owned(),
    }
}

fn display_list(values: &[&str]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}
