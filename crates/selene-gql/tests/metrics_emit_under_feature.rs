//! Feature-on metrics emission tests.

#![cfg(feature = "metrics")]

use std::sync::{Arc, Mutex};

use metrics::{
    Counter, CounterFn, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
};
use selene_core::{GraphId, metrics as selene_metrics};
use selene_gql::{EmptyProcedureRegistry, Session};
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
