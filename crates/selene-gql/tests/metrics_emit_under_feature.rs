//! Feature-on metrics emission tests.

#![cfg(feature = "metrics")]

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use metrics::{
    Counter, CounterFn, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
};
use selene_core::{GraphId, IStr, Value, intern, metrics as selene_metrics};
use selene_gql::{
    CallPlanCache, EmptyProcedureRegistry, GqlType, ProcedureContext, ProcedureError,
    ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    Session,
};
use selene_graph::SharedGraph;

#[derive(Default)]
struct RecordingRecorder {
    counters: Arc<Mutex<Vec<CounterEvent>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CounterEvent {
    name: String,
    labels: Vec<(String, String)>,
    value: u64,
}

impl RecordingRecorder {
    fn counter_events(&self) -> Vec<CounterEvent> {
        self.counters.lock().expect("counter events lock").clone()
    }
}

impl Recorder for RecordingRecorder {
    fn describe_counter(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_histogram(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        Counter::from_arc(Arc::new(RecordingCounter {
            key: key.to_retained(),
            events: Arc::clone(&self.counters),
        }))
    }

    fn register_gauge(&self, _key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        Gauge::noop()
    }

    fn register_histogram(&self, _key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        Histogram::noop()
    }
}

struct RecordingCounter {
    key: Key,
    events: Arc<Mutex<Vec<CounterEvent>>>,
}

impl CounterFn for RecordingCounter {
    fn increment(&self, value: u64) {
        self.events
            .lock()
            .expect("counter events lock")
            .push(CounterEvent {
                name: self.key.name().to_owned(),
                labels: self
                    .key
                    .labels()
                    .map(|label| (label.key().to_owned(), label.value().to_owned()))
                    .collect(),
                value,
            });
    }

    fn absolute(&self, value: u64) {
        self.increment(value);
    }
}

struct MetricsProcedureRegistry {
    name: Box<[IStr]>,
    metadata: ProcedureMetadata,
}

impl MetricsProcedureRegistry {
    fn new() -> Self {
        let name = Box::from([istr("metrics"), istr("noop")]);
        Self {
            name,
            metadata: ProcedureMetadata::new(
                ProcedureHandle::new(1),
                ProcedureSignature::new(Vec::new()),
                ProcedureOutputSchema {
                    columns: vec![ProcedureOutputColumn::new(istr("n"), GqlType::Integer)],
                },
                ProcedureTier::Graph,
                ProcedureMutability::Read,
                None,
            ),
        }
    }
}

impl ProcedureRegistry for MetricsProcedureRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        (name == self.name.as_ref()).then(|| self.metadata.clone())
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        Ok(ProcedureResult {
            rows: vec![vec![Value::Int(1)]],
        })
    }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

#[test]
fn executing_query_increments_query_counter() {
    let recorder = RecordingRecorder::default();

    metrics::with_local_recorder(&recorder, || {
        let graph = SharedGraph::new(GraphId::new(12_100));
        let mut session = Session::new(&graph);
        session
            .execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)
            .expect("query executes");
    });

    let events = recorder.counter_events();
    let query_events: Vec<_> = events
        .iter()
        .filter(|event| event.name == selene_metrics::QUERIES_TOTAL)
        .collect();

    assert_eq!(query_events.len(), 1);
    assert_eq!(query_events[0].value, 1);
    assert_eq!(
        query_events[0].labels,
        vec![(
            selene_metrics::STATEMENT_KIND_LABEL.to_owned(),
            "query".to_owned()
        )]
    );
}

#[test]
fn call_plan_cache_hit_increments_call_cache_counter() {
    let recorder = RecordingRecorder::default();

    metrics::with_local_recorder(&recorder, || {
        let graph = SharedGraph::new(GraphId::new(12_101));
        let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(8).expect("nonzero")));
        let registry = MetricsProcedureRegistry::new();
        Session::new(&graph)
            .with_call_plan_cache(Arc::clone(&cache))
            .execute_source("CALL metrics.noop() YIELD n", &registry)
            .expect("first call executes");
        Session::new(&graph)
            .with_call_plan_cache(cache)
            .execute_source("CALL metrics.noop() YIELD n", &registry)
            .expect("second call executes");
    });

    let events = recorder.counter_events();
    let call_cache_events: Vec<_> = events
        .iter()
        .filter(|event| event.name == selene_metrics::CALL_PLAN_CACHE_HITS_TOTAL)
        .collect();

    assert_eq!(call_cache_events.len(), 1);
    assert_eq!(call_cache_events[0].value, 1);
    assert!(call_cache_events[0].labels.is_empty());
}
