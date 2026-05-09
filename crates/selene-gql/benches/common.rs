#![allow(missing_docs)]
#![allow(dead_code)]

use std::time::Duration;

use criterion::Criterion;
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ImplDefinedCaps, OptimizeContext, ProcedureRegistry,
    analyze, optimize, parse, plan,
};
use selene_testing::{BenchProfile, PlanCorpusCategory};
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
