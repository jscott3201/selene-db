#![allow(missing_docs)]
//! Criterion benches for commit-time index-provider fanout.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::sync::Arc;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{LabelSet, PropertyMap, intern};
use selene_graph::{IndexProvider, SharedGraph};
use selene_testing::BenchFixture;

fn bench_provider_fanout(c: &mut Criterion) {
    let mut group = c.benchmark_group("provider_fanout");
    let fixture = BenchFixture::build(10_000);
    group.throughput(Throughput::Elements(1));

    for case in [
        ProviderCase::new("core_only", 0, ProviderTail::None),
        ProviderCase::new("extra_k1", 1, ProviderTail::None),
        ProviderCase::new("extra_k4", 4, ProviderTail::None),
        ProviderCase::new("extra_k16", 16, ProviderTail::None),
        ProviderCase::new("extra_k4_with_error_one", 4, ProviderTail::Error),
    ] {
        bench_case(&mut group, &fixture, case);
    }

    if std::env::var_os("SELENE_BENCH_INCLUDE_PANIC_PROVIDER").is_some() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        bench_case(
            &mut group,
            &fixture,
            ProviderCase::new("extra_k4_with_panic_one", 4, ProviderTail::Panic),
        );
        std::panic::set_hook(previous);
    }

    group.finish();
}

fn bench_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &BenchFixture,
    case: ProviderCase,
) {
    let label = LabelSet::single(intern("FanoutNode").expect("bench label interns"));
    group.bench_function(BenchmarkId::from_parameter(case.name), |b| {
        b.iter_batched(
            || {
                SharedGraph::from_graph_with_providers(
                    fixture.graph().clone(),
                    providers(case.extra_noop_count, case.tail),
                )
                .expect("provider fixture graph builds")
            },
            |shared| {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    mutator
                        .create_node(label.clone(), PropertyMap::new())
                        .expect("node create succeeds");
                }
                let changes = txn.commit().expect("commit succeeds").changes.len();
                std::hint::black_box((shared, changes))
            },
            BatchSize::SmallInput,
        );
    });
}

fn providers(extra_noop_count: usize, tail: ProviderTail) -> Vec<Arc<dyn IndexProvider>> {
    let mut providers = Vec::<Arc<dyn IndexProvider>>::with_capacity(extra_noop_count + 1);
    for idx in 0..extra_noop_count {
        providers.push(Arc::new(common::noop_provider(common::provider_tag(
            idx + 1,
        ))));
    }
    match tail {
        ProviderTail::None => {}
        ProviderTail::Error => providers.push(Arc::new(common::error_provider(
            common::provider_tag(extra_noop_count + 1),
        ))),
        ProviderTail::Panic => providers.push(Arc::new(common::panicking_provider(
            common::provider_tag(extra_noop_count + 1),
        ))),
    }
    providers
}

#[derive(Clone, Copy)]
struct ProviderCase {
    name: &'static str,
    extra_noop_count: usize,
    tail: ProviderTail,
}

impl ProviderCase {
    const fn new(name: &'static str, extra_noop_count: usize, tail: ProviderTail) -> Self {
        Self {
            name,
            extra_noop_count,
            tail,
        }
    }
}

#[derive(Clone, Copy)]
enum ProviderTail {
    None,
    Error,
    Panic,
}

criterion_group! {
    name = provider_fanout;
    config = common::criterion_config();
    targets = bench_provider_fanout
}
criterion_main!(provider_fanout);
