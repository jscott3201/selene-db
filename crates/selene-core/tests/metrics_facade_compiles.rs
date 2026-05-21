//! Compile smoke tests for the optional metrics facade.

use selene_core::metrics;

#[test]
fn metrics_facade_helpers_compile_without_recorder() {
    metrics::counter_inc(metrics::QUERIES_TOTAL);
    metrics::counter_add(metrics::WAL_APPENDS_TOTAL, 2);
    metrics::counter_inc_with_label(
        metrics::QUERIES_TOTAL,
        metrics::Label::new(metrics::STATEMENT_KIND_LABEL, "query"),
    );
    metrics::histogram_record(metrics::QUERY_DURATION_SECONDS, 0.001);
    metrics::histogram_record_with_label(
        metrics::QUERY_DURATION_SECONDS,
        0.001,
        metrics::Label::new(metrics::STATEMENT_KIND_LABEL, "query"),
    );
    metrics::gauge_set(metrics::GRAPH_NODES, 3.0);
}
