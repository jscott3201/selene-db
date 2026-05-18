#![allow(missing_docs)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use criterion::Criterion;
use parking_lot::Mutex;
use selene_core::{Change, HlcTimestamp, LabelSet, Origin, PropertyMap, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ImplDefinedCaps, OptimizeContext, ProcedureRegistry,
    Session, StatementOutput, analyze, execute_statement, optimize, parse, plan,
};
use selene_graph::{CommitOutcome, IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig, WalWriter};
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
    pub(crate) writer: WalWriter,
    pub(crate) next_hlc: u64,
    _dir: tempfile::TempDir,
}

pub(crate) fn gql_write_state(scale: usize, sync_policy: SyncPolicy) -> GqlWriteState {
    let fixture = BenchFixture::build(scale);
    let dir = tempfile::tempdir().expect("temp dir is created");
    let provider = WalAppendProvider::open(dir.path(), sync_policy);
    let graph = SharedGraph::from_graph_with_providers(
        fixture.graph().clone(),
        vec![std::sync::Arc::new(provider)],
    )
    .expect("GQL write bench graph builds");
    GqlWriteState { graph, _dir: dir }
}

pub(crate) fn direct_write_state(scale: usize, sync_policy: SyncPolicy) -> DirectWriteState {
    let fixture = BenchFixture::build(scale);
    let dir = tempfile::tempdir().expect("temp dir is created");
    let writer = WalWriter::open(
        &dir.path().join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy,
            snapshot_seq: 0,
        },
    )
    .expect("wal opens");
    DirectWriteState {
        graph: SharedGraph::from_graph(fixture.graph().clone()),
        writer,
        next_hlc: 1,
        _dir: dir,
    }
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
        StatementOutput::Empty => 0,
        _ => panic!("unexpected statement output"),
    }
}

pub(crate) fn execute_direct_insert(state: &mut DirectWriteState, flush: bool) -> usize {
    let outcome = commit_direct_insert(&state.graph);
    append_outcome(&mut state.writer, &mut state.next_hlc, &outcome);
    if flush {
        state.writer.flush().expect("wal flush succeeds");
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

fn append_outcome(writer: &mut WalWriter, next_hlc: &mut u64, outcome: &CommitOutcome) {
    writer
        .append(
            HlcTimestamp::new(*next_hlc, 0),
            Origin::Local,
            outcome.principal.clone(),
            &outcome.changes,
        )
        .expect("wal append succeeds");
    *next_hlc += 1;
}

struct WalAppendProvider {
    writer: Mutex<WalWriter>,
    next_hlc: AtomicU64,
}

impl WalAppendProvider {
    fn open(dir: &std::path::Path, sync_policy: SyncPolicy) -> Self {
        let writer = WalWriter::open(
            &dir.join(DEFAULT_WAL_FILE_NAME),
            WalConfig {
                sync_policy,
                snapshot_seq: 0,
            },
        )
        .expect("wal opens");
        Self {
            writer: Mutex::new(writer),
            next_hlc: AtomicU64::new(1),
        }
    }
}

impl IndexProvider for WalAppendProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"BWAL")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        let hlc = self.next_hlc.fetch_add(1, Ordering::Relaxed);
        let mut writer = self.writer.lock();
        writer
            .append(
                HlcTimestamp::new(hlc, 0),
                Origin::Local,
                None,
                std::slice::from_ref(change),
            )
            .map(|_| ())
            .map_err(|error| ProviderError::Inconsistent {
                reason: format!("bench WAL append failed: {error}"),
            })
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}
