//! Pinned `since_version` release-honesty surface.
//!
//! `since_version` is the compatibility claim downstream consumers gate on:
//! it must name the first release tag that shipped the procedure, not the
//! cycle the registration entry was authored in. The pinned partition below
//! is grounded in tag evidence:
//!
//! - `git ls-tree v1.1.0 --name-only -r crates/selene-gql/src/runtime/builtins/`
//!   shows only `health`, `feature_status`, `verify`, `create_index`, and
//!   `drop_index` existed at the v1.1.0 tag; the next 41 `selene.*`
//!   built-ins ship first in v1.2.0, the P1 BM25 candidate-state procedures
//!   ship first in v1.3.0, and `selene.reachable_nodes` ships first in v1.4.0.
//! - `git show v1.0.0:crates/selene-algorithms-pack/src/registry.rs`
//!   registered all 19 `algo.*` procedures, so that surface has been
//!   available since v1.0.0 (native in-tree since v1.1.0).
//!
//! Adding or re-versioning a procedure must consciously extend this
//! partition; a stale or recycled claim fails the per-name assertion or the
//! exact bucket counts.

use selene_core::{DbString, GraphId, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn column_strings(table: &BindingTable, name: &str) -> Vec<String> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
        })
        .collect()
}

/// First release tag that shipped each procedure, keyed by rendered name.
///
/// The `"1.2.0"` arm is intentionally a catch-all: the exact bucket counts in
/// [`registry_since_version_matches_release_history`] reject any procedure
/// riding it without updating this pin.
fn expected_since_version(rendered: &str) -> &'static str {
    match rendered {
        name if name.starts_with("algo.") => "1.0.0",
        "selene.health" | "selene.create_index" | "selene.drop_index" => "1.0.0",
        "selene.feature_status" | "selene.verify" => "1.1.0",
        "selene.text_score_candidate_state" | "selene.text_score_candidate_state_nodes" => "1.3.0",
        "selene.reachable_nodes" => "1.4.0",
        _ => "1.2.0",
    }
}

#[test]
fn registry_since_version_matches_release_history() {
    let registry = BuiltinProcedureRegistry::new();
    // Buckets index the pinned release partition:
    // ["1.0.0", "1.1.0", "1.2.0", "1.3.0", "1.4.0"].
    let mut buckets = [0_usize; 5];
    let mut algo = 0_usize;
    let mut selene = 0_usize;

    for (name, metadata) in registry.iter_handles() {
        let rendered = name
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(".");
        match name.first().map(DbString::as_str) {
            Some("algo") => algo += 1,
            Some("selene") => selene += 1,
            other => panic!("unexpected procedure namespace {other:?} for {rendered}"),
        }
        let expected = expected_since_version(&rendered);
        assert_eq!(
            metadata.signature.since_version, expected,
            "{rendered} must claim the first release tag that shipped it"
        );
        match expected {
            "1.0.0" => buckets[0] += 1,
            "1.1.0" => buckets[1] += 1,
            "1.2.0" => buckets[2] += 1,
            "1.3.0" => buckets[3] += 1,
            "1.4.0" => buckets[4] += 1,
            other => panic!("unexpected since_version bucket {other}"),
        }
    }

    // 19 `algo.*` plus `health`/`create_index`/`drop_index` shipped in
    // v1.0.0; `feature_status`/`verify` shipped in v1.1.0; 41 built-ins ship
    // first in v1.2.0; the two BM25 candidate-state procedures ship first in
    // v1.3.0; and reachable_nodes ships first in v1.4.0. A new or re-versioned
    // procedure must extend this partition explicitly.
    assert_eq!(algo, 19);
    assert_eq!(selene, 49);
    assert_eq!(buckets, [22, 2, 41, 2, 1]);
}

#[test]
fn show_procedures_since_version_column_matches_release_history() {
    let graph = SharedGraph::new(GraphId::new(121_001));
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let table = rows(
        session
            .execute_source("SHOW PROCEDURES", &registry)
            .expect("SHOW PROCEDURES executes"),
    );
    assert_eq!(table.row_count(), 68);

    // The planner-rendered column must agree with the registry claim per
    // row; divergence here means the SHOW plumbing dropped or rewrote the
    // signature metadata.
    let names = column_strings(&table, "name");
    let since_versions = column_strings(&table, "since_version");
    for (name, since) in names.iter().zip(&since_versions) {
        assert_eq!(
            since,
            expected_since_version(name),
            "SHOW PROCEDURES since_version for {name}"
        );
    }
}
