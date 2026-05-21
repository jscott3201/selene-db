//! Optional metrics facade for embedders that install a `metrics` recorder.
//!
//! The helpers in this module are always available. With the `metrics`
//! feature enabled they emit through the upstream `metrics` facade; with the
//! feature disabled they are no-ops, so call sites do not need conditional
//! compilation.

/// Total executed statements.
pub const QUERIES_TOTAL: &str = "selene.queries.total";
/// Statement execution latency, in seconds.
pub const QUERY_DURATION_SECONDS: &str = "selene.query.duration_seconds";
/// Total successful graph commits.
pub const COMMITS_TOTAL: &str = "selene.commits.total";
/// Graph commit latency, in seconds.
pub const COMMIT_DURATION_SECONDS: &str = "selene.commit.duration_seconds";
/// Total WAL appends.
pub const WAL_APPENDS_TOTAL: &str = "selene.wal.appends.total";
/// Total finalized snapshots.
pub const SNAPSHOTS_TOTAL: &str = "selene.snapshots.total";
/// Snapshot finalization latency, in seconds.
pub const SNAPSHOT_DURATION_SECONDS: &str = "selene.snapshot.duration_seconds";
/// Total recovery passes.
pub const RECOVERIES_TOTAL: &str = "selene.recoveries.total";
/// Recovery pass latency, in seconds.
pub const RECOVERY_DURATION_SECONDS: &str = "selene.recovery.duration_seconds";
/// Total cooperative cancellation events.
pub const CANCELLATIONS_TOTAL: &str = "selene.cancellations.total";
/// Total vector search procedure calls.
pub const VECTOR_SEARCHES_TOTAL: &str = "selene.vector.searches.total";
/// Total algorithm procedure calls.
pub const ALGORITHM_RUNS_TOTAL: &str = "selene.algorithm.runs.total";
/// Current live node count.
pub const GRAPH_NODES: &str = "selene.graph.nodes";
/// Current live edge count.
pub const GRAPH_EDGES: &str = "selene.graph.edges";
/// Low-cardinality statement-kind label key.
pub const STATEMENT_KIND_LABEL: &str = "statement.kind";

/// Static metric label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Label {
    /// Label key.
    pub key: &'static str,
    /// Label value.
    pub value: &'static str,
}

impl Label {
    /// Construct a static metric label.
    #[must_use]
    pub const fn new(key: &'static str, value: &'static str) -> Self {
        Self { key, value }
    }
}

/// Increment a counter by one.
pub fn counter_inc(name: &'static str) {
    counter_add(name, 1);
}

/// Increment a counter by one with a single static label.
pub fn counter_inc_with_label(name: &'static str, label: Label) {
    counter_add_with_label(name, 1, label);
}

/// Add `value` to a counter.
#[cfg(feature = "metrics")]
pub fn counter_add(name: &'static str, value: u64) {
    metrics::counter!(name).increment(value);
}

/// Add `value` to a counter.
#[cfg(not(feature = "metrics"))]
pub fn counter_add(_name: &'static str, _value: u64) {}

/// Add `value` to a counter with a single static label.
#[cfg(feature = "metrics")]
pub fn counter_add_with_label(name: &'static str, value: u64, label: Label) {
    metrics::counter!(name, label.key => label.value).increment(value);
}

/// Add `value` to a counter with a single static label.
#[cfg(not(feature = "metrics"))]
pub fn counter_add_with_label(_name: &'static str, _value: u64, _label: Label) {}

/// Record a duration histogram sample in seconds.
#[cfg(feature = "metrics")]
pub fn histogram_record(name: &'static str, seconds: f64) {
    metrics::histogram!(name).record(seconds);
}

/// Record a duration histogram sample in seconds.
#[cfg(not(feature = "metrics"))]
pub fn histogram_record(_name: &'static str, _seconds: f64) {}

/// Record a duration histogram sample in seconds with a single static label.
#[cfg(feature = "metrics")]
pub fn histogram_record_with_label(name: &'static str, seconds: f64, label: Label) {
    metrics::histogram!(name, label.key => label.value).record(seconds);
}

/// Record a duration histogram sample in seconds with a single static label.
#[cfg(not(feature = "metrics"))]
pub fn histogram_record_with_label(_name: &'static str, _seconds: f64, _label: Label) {}

/// Set a gauge value.
#[cfg(feature = "metrics")]
pub fn gauge_set(name: &'static str, value: f64) {
    metrics::gauge!(name).set(value);
}

/// Set a gauge value.
#[cfg(not(feature = "metrics"))]
pub fn gauge_set(_name: &'static str, _value: f64) {}
