//! M5d executor snapshot corpus harness.

use std::collections::BTreeSet;

use selene_core::GraphId;
use selene_gql::{
    BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan, ImplDefinedCaps,
    NetGraphDelta, OptimizeContext, ProcedureRegistry, RowOrderPolicy, StatementCategory,
    StatementOutput, analyze, execute_statement, executor_summary, optimize, parse, plan,
};
use selene_graph::SharedGraph;
use selene_testing::{
    ExecutorCorpus, ExecutorCorpusCategory, ExecutorCorpusEntry, ExecutorCorpusProgram,
    ExecutorCorpusRegistry, MockProcedureRegistry, PHASE_A_OPERATORS,
};

#[test]
fn corpus_snapshots_match() {
    for (index, entry) in ExecutorCorpus::m5d().entries().enumerate() {
        let executed = execute_entry(index, entry);
        let snapshot = executor_summary(&selene_gql::ExecutorSummaryInput {
            table: &executed.table,
            row_order: row_order_policy(&executed.plan),
            deltas: executed.deltas,
        });

        insta::with_settings!({ snapshot_suffix => entry.slug }, {
            insta::assert_snapshot!(snapshot.to_string());
        });
    }
}

#[test]
fn corpus_slugs_are_unique() {
    let mut slugs = BTreeSet::new();
    for entry in ExecutorCorpus::m5d().entries() {
        assert!(slugs.insert(entry.slug), "duplicate slug {}", entry.slug);
    }
}

#[test]
fn corpus_categories_covered() {
    let actual = ExecutorCorpus::m5d()
        .entries()
        .map(|entry| entry.category)
        .collect::<BTreeSet<_>>();
    let expected = [
        ExecutorCorpusCategory::Read,
        ExecutorCorpusCategory::Mutation,
        ExecutorCorpusCategory::Ddl,
        ExecutorCorpusCategory::Catalog,
        ExecutorCorpusCategory::Call,
        ExecutorCorpusCategory::Transaction,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn corpus_covers_every_operator() {
    let actual = ExecutorCorpus::m5d()
        .entries()
        .flat_map(|entry| entry.covered_operators.iter().copied())
        .collect::<BTreeSet<_>>();
    let expected = PHASE_A_OPERATORS.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn corpus_output_schemas_match_runtime_tables() {
    for (index, entry) in ExecutorCorpus::m5d().entries().enumerate() {
        let executed = execute_entry(index, entry);
        assert_eq!(
            executed.plan.output_schema.columns.len(),
            executed.table.schema().columns.len(),
            "{} output schema width drifted",
            entry.slug
        );
    }
}

#[test]
fn corpus_categories_match_plan_categories() {
    for (index, entry) in ExecutorCorpus::m5d().entries().enumerate() {
        let graph = entry.fixture.build(graph_id(index));
        let empty_registry = EmptyProcedureRegistry;
        let mock_registry = ExecutorCorpus::standard_mock_registry();
        let registry = registry_for(entry, &empty_registry, &mock_registry);
        match entry.program {
            ExecutorCorpusProgram::Single(source) => {
                let plan = plan_source(source, &graph, registry, entry.uses_index_catalog);
                assert_category_matches(entry, plan.category);
            }
            ExecutorCorpusProgram::HandBuilt(build) => {
                let plan = optimize_entry(entry, build());
                assert_category_matches(entry, plan.category);
            }
            ExecutorCorpusProgram::Sequence(sources) => {
                let categories = sources
                    .iter()
                    .map(|source| plan_source(source, &graph, registry, false).category)
                    .collect::<Vec<_>>();
                assert_eq!(entry.category, ExecutorCorpusCategory::Transaction);
                assert!(categories.contains(&StatementCategory::TransactionControl));
            }
        }
    }
}

struct ExecutedEntry {
    plan: ExecutionPlan,
    table: BindingTable,
    deltas: NetGraphDelta,
}

fn execute_entry(index: usize, entry: &ExecutorCorpusEntry) -> ExecutedEntry {
    let graph = entry.fixture.build(graph_id(index));
    let before = graph.read();
    let empty_registry = EmptyProcedureRegistry;
    let mock_registry = ExecutorCorpus::standard_mock_registry();
    let registry = registry_for(entry, &empty_registry, &mock_registry);
    let mut session = selene_gql::Session::new(&graph);

    let (plan, output) = match entry.program {
        ExecutorCorpusProgram::Single(source) => {
            let plan = plan_source(source, &graph, registry, entry.uses_index_catalog);
            let output = execute_statement(&plan, &mut session, registry)
                .unwrap_or_else(|error| panic!("{} failed to execute: {error:?}", entry.slug));
            (plan, output)
        }
        ExecutorCorpusProgram::Sequence(sources) => {
            let mut last = None;
            for source in sources {
                let plan = plan_source(source, &graph, registry, entry.uses_index_catalog);
                let output =
                    execute_statement(&plan, &mut session, registry).unwrap_or_else(|error| {
                        panic!("{} failed to execute `{source}`: {error:?}", entry.slug)
                    });
                last = Some((plan, output));
            }
            last.expect("sequence has at least one statement")
        }
        ExecutorCorpusProgram::HandBuilt(build) => {
            let plan = optimize_entry(entry, build());
            let output = execute_statement(&plan, &mut session, registry)
                .unwrap_or_else(|error| panic!("{} failed to execute: {error:?}", entry.slug));
            (plan, output)
        }
    };
    let after = graph.read();

    ExecutedEntry {
        table: table_for_output(output),
        plan,
        deltas: NetGraphDelta::between(before.as_ref(), after.as_ref()),
    }
}

fn plan_source(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    uses_index_catalog: bool,
) -> ExecutionPlan {
    let statement =
        parse(source).unwrap_or_else(|error| panic!("failed to parse `{source}`: {error:?}"));
    let snapshot = graph.read();
    let schema = snapshot.meta.bound_type.as_deref();
    let analyzed = analyze(statement, registry, schema)
        .unwrap_or_else(|error| panic!("failed to analyze `{source}`: {error:?}"));
    let plan = plan(&analyzed, registry)
        .unwrap_or_else(|error| panic!("failed to plan `{source}`: {error:?}"));
    optimize_with_catalog(plan, uses_index_catalog)
}

fn optimize_entry(entry: &ExecutorCorpusEntry, plan: ExecutionPlan) -> ExecutionPlan {
    optimize_with_catalog(plan, entry.uses_index_catalog)
}

fn optimize_with_catalog(plan: ExecutionPlan, uses_index_catalog: bool) -> ExecutionPlan {
    let caps = ImplDefinedCaps::default();
    let catalog = ExecutorCorpus::standard_mock_catalog();
    let ctx = if uses_index_catalog {
        OptimizeContext::new(&caps).with_index_catalog(&catalog)
    } else {
        OptimizeContext::new(&caps)
    };
    optimize(plan, &ctx)
}

fn table_for_output(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.unwrap_or_else(|| {
            BindingTable::empty(BindingTableSchema {
                columns: Vec::new(),
            })
        }),
        _ => BindingTable::empty(BindingTableSchema {
            columns: Vec::new(),
        }),
    }
}

fn registry_for<'a>(
    entry: &ExecutorCorpusEntry,
    empty_registry: &'a EmptyProcedureRegistry,
    mock_registry: &'a MockProcedureRegistry,
) -> &'a dyn ProcedureRegistry {
    match entry.registry {
        ExecutorCorpusRegistry::Empty => empty_registry,
        ExecutorCorpusRegistry::StandardMock => mock_registry,
        _ => empty_registry,
    }
}

fn assert_category_matches(entry: &ExecutorCorpusEntry, actual: StatementCategory) {
    let expected = match entry.category {
        ExecutorCorpusCategory::Read
        | ExecutorCorpusCategory::Catalog
        | ExecutorCorpusCategory::Call => StatementCategory::ReadOnly,
        ExecutorCorpusCategory::Mutation => StatementCategory::DataModifying,
        ExecutorCorpusCategory::Ddl => StatementCategory::CatalogModifying,
        ExecutorCorpusCategory::Transaction => StatementCategory::TransactionControl,
        _ => StatementCategory::ReadOnly,
    };
    assert_eq!(actual, expected, "{} category drifted", entry.slug);
}

fn row_order_policy(plan: &ExecutionPlan) -> RowOrderPolicy {
    if plan_has_ordered_boundary(plan) {
        RowOrderPolicy::PreserveEmitted
    } else {
        RowOrderPolicy::SortDeterministic
    }
}

fn plan_has_ordered_boundary(plan: &ExecutionPlan) -> bool {
    plan.pipeline.iter().any(|op| {
        matches!(
            op,
            selene_gql::PipelineOp::OrderBy(_) | selene_gql::PipelineOp::TopK { .. }
        )
    })
}

fn graph_id(index: usize) -> GraphId {
    GraphId::new(40_000 + index as u64)
}

#[test]
fn unordered_snapshot_placeholder_assignment_is_stable_after_row_shuffle() {
    use selene_core::{NodeId, Value, db_string};
    use selene_gql::{AnalyzedType, Binding, BindingTableColumn};

    let schema = BindingTableSchema {
        columns: vec![BindingTableColumn {
            name: Some(db_string("n").expect("test string fits DB string cap")),
            hidden: None,
            ty: AnalyzedType::Dynamic,
        }],
    };
    let lhs = BindingTable::new(
        schema.clone(),
        vec![
            Binding::new([Value::NodeRef(NodeId::new(2))]),
            Binding::new([Value::NodeRef(NodeId::new(1))]),
        ],
    );
    let rhs = BindingTable::new(
        schema,
        vec![
            Binding::new([Value::NodeRef(NodeId::new(1))]),
            Binding::new([Value::NodeRef(NodeId::new(2))]),
        ],
    );
    assert_eq!(summary_for(&lhs), summary_for(&rhs));
}

fn summary_for(table: &BindingTable) -> selene_gql::ExecutorSnapshot {
    executor_summary(&selene_gql::ExecutorSummaryInput {
        table,
        row_order: RowOrderPolicy::SortDeterministic,
        deltas: NetGraphDelta::default(),
    })
}
