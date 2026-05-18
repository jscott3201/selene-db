#![allow(missing_docs)]
#![allow(dead_code)]

use std::time::Duration;

use criterion::Criterion;
use selene_core::{LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ImplDefinedCaps, OptimizeContext, ProcedureRegistry,
    Session, StatementOutput, analyze, execute_statement, optimize, parse, plan,
};
use selene_graph::{CommitOutcome, SharedGraph};
use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig};
use selene_testing::{BenchFixture, BenchProfile, PlanCorpusCategory};
use selene_testing::{MockIndexCatalog, MockProcedureRegistry};
use selene_testing::{PlanCorpus, PlanCorpusEntry, PlanCorpusRegistry};

pub(crate) fn criterion_config() -> Criterion {
    let profile = BenchProfile::from_env();
    Criterion::default()
        .sample_size(profile.sample_size())
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(match profile {
            BenchProfile::Quick => 500,
            BenchProfile::Full | BenchProfile::Stress => 1_500,
            _ => 500,
        }))
}

pub(crate) fn corpus_entries() -> Vec<PlanCorpusEntry> {
    PlanCorpus::m5c().entries().cloned().collect()
}

pub(crate) fn registry_for<'a>(
    entry: &PlanCorpusEntry,
    empty: &'a EmptyProcedureRegistry,
    mock: &'a MockProcedureRegistry,
) -> &'a dyn ProcedureRegistry {
    match entry.registry {
        PlanCorpusRegistry::Empty => empty,
        PlanCorpusRegistry::StandardMock => mock,
        _ => empty,
    }
}

pub(crate) fn context_for<'a>(
    entry: &PlanCorpusEntry,
    caps: &'a ImplDefinedCaps,
    catalog: &'a MockIndexCatalog,
) -> OptimizeContext<'a> {
    let ctx = OptimizeContext::new(caps);
    if entry.uses_index_catalog {
        ctx.with_index_catalog(catalog)
    } else {
        ctx
    }
}

pub(crate) fn plan_and_optimize_entry(
    entry: &PlanCorpusEntry,
    empty: &EmptyProcedureRegistry,
    mock: &MockProcedureRegistry,
    catalog: &MockIndexCatalog,
) -> ExecutionPlan {
    let registry = registry_for(entry, empty, mock);
    let statement = parse(entry.source).expect("corpus source parses");
    let analyzed = analyze(statement, registry, None).expect("corpus source analyzes");
    let planned = plan(&analyzed, registry).expect("corpus source plans");
    let caps = ImplDefinedCaps::default();
    let ctx = context_for(entry, &caps, catalog);
    optimize(planned, &ctx)
}

pub(crate) fn representative_plan() -> ExecutionPlan {
    let entries = corpus_entries();
    let entry = entries
        .iter()
        .find(|entry| entry.category == PlanCorpusCategory::Read && entry.uses_index_catalog)
        .expect("corpus has an index-aware read entry");
    let empty = EmptyProcedureRegistry;
    let mock = PlanCorpus::standard_mock_registry();
    let catalog = PlanCorpus::standard_mock_catalog();
    plan_and_optimize_entry(entry, &empty, &mock, &catalog)
}

pub(crate) struct GqlWriteState {
    pub(crate) graph: SharedGraph,
    _dir: tempfile::TempDir,
}

pub(crate) struct DirectWriteState {
    pub(crate) graph: SharedGraph,
    _dir: tempfile::TempDir,
}

pub(crate) fn gql_write_state(scale: usize, sync_policy: SyncPolicy) -> GqlWriteState {
    let fixture = BenchFixture::build(scale);
    let dir = tempfile::tempdir().expect("temp dir is created");
    let graph = SharedGraph::from_graph_with_wal(
        fixture.graph().clone(),
        dir.path().join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy,
            snapshot_seq: 0,
        },
    )
    .expect("GQL write bench graph builds");
    GqlWriteState { graph, _dir: dir }
}

pub(crate) fn direct_write_state(scale: usize, sync_policy: SyncPolicy) -> DirectWriteState {
    let fixture = BenchFixture::build(scale);
    let dir = tempfile::tempdir().expect("temp dir is created");
    let graph = SharedGraph::from_graph_with_wal(
        fixture.graph().clone(),
        dir.path().join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy,
            snapshot_seq: 0,
        },
    )
    .expect("direct write bench graph builds");
    DirectWriteState { graph, _dir: dir }
}

pub(crate) fn plan_write(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("write source parses");
    let analyzed =
        analyze(statement, &EmptyProcedureRegistry, None).expect("write source analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("write source plans")
}

pub(crate) fn execute_preplanned(plan: &ExecutionPlan, session: &mut Session<'_>) -> usize {
    match execute_statement(plan, session, &EmptyProcedureRegistry)
        .expect("write statement executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        StatementOutput::Written(outcome) => {
            let rows = outcome.rows.as_ref().map_or(0, |table| table.row_count());
            rows + outcome.changes.len()
        }
        StatementOutput::Empty => 0,
        _ => panic!("unexpected statement output"),
    }
}

pub(crate) fn execute_direct_insert(state: &mut DirectWriteState, flush: bool) -> usize {
    let outcome = commit_direct_insert(&state.graph);
    if flush {
        Session::new(&state.graph)
            .flush()
            .expect("durable provider flush succeeds");
    }
    outcome.changes.len()
}

fn commit_direct_insert(graph: &SharedGraph) -> CommitOutcome {
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        let name = intern("name").expect("name key interns");
        let score = intern("score").expect("score key interns");
        mutator
            .create_node(
                LabelSet::single(intern("Person").expect("Person label interns")),
                PropertyMap::from_pairs([
                    (name, Value::String(intern("x").expect("x value interns"))),
                    (score, Value::Int(42)),
                ])
                .expect("direct insert properties fit"),
            )
            .expect("direct node create succeeds");
    }
    txn.commit().expect("direct commit succeeds")
}
