#![allow(missing_docs)]
//! Criterion benches for commit-time index-provider fanout.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{Change, EdgeId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_testing::BenchFixture;

const ACTIVE_SET_BATCH: usize = 40;

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

    bench_active_set_edge_create(&mut group, &fixture);
    bench_active_set_edge_delete(&mut group, &fixture);

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

fn bench_active_set_edge_create(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &BenchFixture,
) {
    let label = active_set_edge_label();
    let sources = active_sources();
    group.throughput(Throughput::Elements(ACTIVE_SET_BATCH as u64));
    group.bench_function(
        BenchmarkId::from_parameter("active_set_edge_create_k40"),
        |b| {
            b.iter_batched(
                || active_set_shared(fixture, &label, &sources),
                |(shared, provider)| {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        for (idx, source) in sources.iter().copied().enumerate() {
                            mutator
                                .create_edge(
                                    label.clone(),
                                    source,
                                    active_target(idx),
                                    PropertyMap::new(),
                                )
                                .expect("active-set edge create succeeds");
                        }
                    }
                    let changes = txn.commit().expect("commit succeeds").changes.len();
                    std::hint::black_box((shared, changes, provider.active_len()))
                },
                BatchSize::SmallInput,
            );
        },
    );
}

fn bench_active_set_edge_delete(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &BenchFixture,
) {
    let label = active_set_edge_label();
    let sources = active_sources();
    group.throughput(Throughput::Elements(ACTIVE_SET_BATCH as u64));
    group.bench_function(
        BenchmarkId::from_parameter("active_set_edge_delete_k40"),
        |b| {
            b.iter_batched(
                || {
                    let (shared, provider) = active_set_shared(fixture, &label, &sources);
                    let mut seeded_edges = Vec::with_capacity(ACTIVE_SET_BATCH);
                    {
                        let mut txn = shared.begin_write();
                        {
                            let mut mutator = txn.mutator();
                            for (idx, source) in sources.iter().copied().enumerate() {
                                seeded_edges.push(
                                    mutator
                                        .create_edge(
                                            label.clone(),
                                            source,
                                            active_target(idx),
                                            PropertyMap::new(),
                                        )
                                        .expect("active-set seed edge create succeeds"),
                                );
                            }
                        }
                        txn.commit().expect("seed commit succeeds");
                    }
                    (shared, provider, seeded_edges)
                },
                |(shared, provider, seeded_edges)| {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        for edge_id in seeded_edges {
                            mutator
                                .delete_edge(edge_id)
                                .expect("active-set edge delete succeeds");
                        }
                    }
                    let changes = txn.commit().expect("commit succeeds").changes.len();
                    std::hint::black_box((shared, changes, provider.active_len()))
                },
                BatchSize::SmallInput,
            );
        },
    );
}

fn active_set_shared(
    fixture: &BenchFixture,
    label: &IStr,
    sources: &[NodeId],
) -> (SharedGraph, Arc<ActiveSetProvider>) {
    let provider = Arc::new(ActiveSetProvider::new(label.clone(), sources));
    let shared = SharedGraph::from_graph_with_providers(
        fixture.graph().clone(),
        vec![provider.clone() as Arc<dyn IndexProvider>],
    )
    .expect("active-set provider fixture builds");
    (shared, provider)
}

fn active_set_edge_label() -> IStr {
    intern("ACTIVE_SET_CONTRADICTS").expect("bench edge label interns")
}

fn active_sources() -> Vec<NodeId> {
    (1..=ACTIVE_SET_BATCH as u64).map(NodeId::new).collect()
}

fn active_target(idx: usize) -> NodeId {
    NodeId::new(10_000 - idx as u64)
}

struct ActiveSetProvider {
    label: IStr,
    state: Mutex<ActiveSetState>,
}

struct ActiveSetState {
    active: HashSet<NodeId>,
    edge_sources: HashMap<EdgeId, NodeId>,
}

impl ActiveSetProvider {
    fn new(label: IStr, active_nodes: &[NodeId]) -> Self {
        Self {
            label,
            state: Mutex::new(ActiveSetState {
                active: active_nodes.iter().copied().collect(),
                edge_sources: HashMap::with_capacity(active_nodes.len()),
            }),
        }
    }

    fn active_len(&self) -> usize {
        self.state
            .lock()
            .expect("active-set mutex poisoned")
            .active
            .len()
    }
}

impl IndexProvider for ActiveSetProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"ASET")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        let mut state = self.state.lock().expect("active-set mutex poisoned");
        match change {
            Change::EdgeCreated {
                id, label, source, ..
            } if label == &self.label => {
                state.edge_sources.insert(*id, *source);
                state.active.remove(source);
            }
            Change::EdgeDeleted { id } => {
                if let Some(source) = state.edge_sources.remove(id) {
                    state.active.insert(source);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
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
