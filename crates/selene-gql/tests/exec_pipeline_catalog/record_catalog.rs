//! Closed-record catalog end-to-end tests (JSON/L1c-d): typed RECORD property DDL lowering
//! and DATA-write validation through the full GQL pipeline — the analyzer → lowering →
//! runtime seam that the analyzer-only and graph-layer tests do not exercise together.
//! Included via `#[path]` from `exec_pipeline_catalog.rs` to keep that test root under the
//! 700-LOC cap; reuses the parent binary's `planned`/`run_write`/`empty_closed_graph`.

use super::{empty_closed_graph, planned, run_write};

#[test]
fn closed_record_property_type_lowers_end_to_end() {
    // Full grammar -> builder -> analyzer -> lowering -> closed-graph commit for a typed
    // RECORD declaration.
    let graph = empty_closed_graph(3717);
    let plan = planned("CREATE NODE TYPE :Host (config :: RECORD{host :: STRING, port :: INT})");
    let (_table, outcome) = run_write(&graph, &plan).expect("closed RECORD type executes");
    outcome.expect("closed RECORD property type commits");
}

#[test]
fn record_value_data_write_validates_against_closed_record_property() {
    // Declare the typed RECORD property, then DATA-write record values into it. The
    // conforming value must analyze AND commit (regression guard for the analyzer/runtime
    // divergence that made typed-RECORD writes unexecutable); the non-conforming value is
    // rejected at commit with G2000 (C7 enforcement through the real query interface).
    let graph = empty_closed_graph(3718);

    let ddl = planned("CREATE NODE TYPE :Host (config :: RECORD{host :: STRING, port :: INT})");
    run_write(&graph, &ddl)
        .expect("record type DDL executes")
        .1
        .expect("record type DDL commits");

    let conforming = planned("INSERT (n:Host {config: RECORD{host: 'h', port: 1}})");
    run_write(&graph, &conforming)
        .expect("conforming record write executes")
        .1
        .expect("conforming record value commits");

    let violating = planned("INSERT (n:Host {config: RECORD{host: 'h', port: 'not-an-int'}})");
    let (_table, outcome) = run_write(&graph, &violating).expect("violating record write executes");
    let error = outcome.expect_err("non-conforming record value is rejected at commit");
    assert_eq!(error.gqlstatus(), "G2000");
}
