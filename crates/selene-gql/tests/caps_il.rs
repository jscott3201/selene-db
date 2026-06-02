//! CAPS-IL: caller-configurable implementation-defined caps through the public
//! `Session` surface (ISO IL013/IL015/IL018 limit surfaces).
//!
//! These tests exercise the embedder-facing path
//! [`Session::with_impl_defined_caps`] end to end via `execute_source` — proving
//! the configured caps reach (a) the plan-time variable-length quantifier gate,
//! (b) the same gate *inside* a subquery body, and (c) the runtime cap checks.
//! Before this wiring an embedder had no path to set non-default caps; the
//! quantifier gate read `ImplDefinedCaps::default()` directly, so a caller value
//! could not loosen or tighten it.

use std::num::NonZeroUsize;

use selene_core::GraphId;
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, ImplDefinedCaps, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

/// A low `max_quantifier` set on the session is honored by the plan-time gate:
/// a bounded quantifier whose upper bound exceeds the cap is rejected at plan
/// time with `5GQL1`, even though it is far below the default of 100.
#[test]
fn session_max_quantifier_cap_rejects_at_plan_time() {
    let graph = graph(9_801);
    let caps = ImplDefinedCaps::DEFAULT.with_max_quantifier(5);
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);

    let err = session
        .execute_source(
            "MATCH (a)-[:K*1..10]->(b) RETURN a",
            &EmptyProcedureRegistry,
        )
        .expect_err("quantifier upper bound 10 exceeds the configured cap of 5");

    assert!(
        matches!(err, ExecutorError::Plan { .. }),
        "expected a plan-time rejection, got {err:?}"
    );
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}

/// The configured cap is the *only* authority: raising it above the default
/// (100) lets a quantifier the default would reject plan and execute. This is
/// the proof that the gate consults the embedder value, not `default()`.
#[test]
fn session_max_quantifier_cap_raises_default_ceiling() {
    let graph = graph(9_802);
    let caps = ImplDefinedCaps::DEFAULT.with_max_quantifier(200);
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);

    // `*1..150` exceeds the default 100 but is within the configured 200.
    let output = session
        .execute_source(
            "MATCH (a)-[:K*1..150]->(b) RETURN a",
            &EmptyProcedureRegistry,
        )
        .expect("quantifier upper bound 150 is within the configured cap of 200");

    // Empty graph: the bounded expansion simply yields no rows.
    match output {
        StatementOutput::Rows(table) => assert_eq!(table.row_count(), 0),
        other => panic!("expected row output, got {other:?}"),
    }
}

/// A bare default-cap session still rejects the same `*1..150` query — confirms
/// the previous test passed because of the raised cap, not because the gate is
/// inert.
#[test]
fn default_session_still_rejects_above_default_ceiling() {
    let graph = graph(9_803);
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "MATCH (a)-[:K*1..150]->(b) RETURN a",
            &EmptyProcedureRegistry,
        )
        .expect_err("150 exceeds the default cap of 100");

    assert!(matches!(err, ExecutorError::Plan { .. }), "got {err:?}");
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}

/// The cap reaches the gate *inside* a subquery body too: a quantifier in an
/// `EXISTS { ... }` pattern is gated against the configured cap. This guards the
/// `populate_plan_subqueries` threading — without it a subquery quantifier would
/// silently fall back to the default cap, an inconsistent limit surface.
#[test]
fn session_max_quantifier_cap_applies_inside_subquery() {
    let graph = graph(9_804);
    let caps = ImplDefinedCaps::DEFAULT.with_max_quantifier(5);
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);

    let err = session
        .execute_source(
            "RETURN EXISTS { MATCH (a)-[:K*1..10]->(b) } AS e",
            &EmptyProcedureRegistry,
        )
        .expect_err("subquery quantifier upper bound 10 exceeds the configured cap of 5");

    assert!(matches!(err, ExecutorError::Plan { .. }), "got {err:?}");
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}

/// A non-quantifier cap (`set_op_key_cap`) also flows through the public path:
/// it is post-stamped onto the plan and consulted by the runtime set-op key
/// counter, so a tiny cap surfaces `5GQL1` at execution.
#[test]
fn session_set_op_key_cap_flows_to_runtime() {
    let graph = graph(9_805);
    let caps =
        ImplDefinedCaps::DEFAULT.with_set_op_key_cap(NonZeroUsize::new(1).expect("non-zero cap"));
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);

    let err = session
        .execute_source(
            "UNWIND [1, 2] AS n RETURN n EXCEPT RETURN 99 AS n",
            &EmptyProcedureRegistry,
        )
        .expect_err("two distinct keys exceed the set-op key cap of 1");

    assert!(
        matches!(
            err,
            ExecutorError::ProgramLimitExceeded {
                detail: "set-op key cap exceeded",
                ..
            }
        ),
        "got {err:?}"
    );
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}

/// A second non-quantifier cap (`group_by_key_cap`) flows through the public
/// path too: post-stamped onto the plan and consulted by the runtime `GROUP BY`
/// distinct-group counter, so a tiny cap surfaces `5GQL1` at execution. Together
/// with the set-op test this proves the post-stamp delivers the embedder value
/// to the runtime cap consumers, not just the plan-time gate.
#[test]
fn session_group_by_key_cap_flows_to_runtime() {
    let graph = graph(9_806);
    let caps =
        ImplDefinedCaps::DEFAULT.with_group_by_key_cap(NonZeroUsize::new(1).expect("non-zero cap"));
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);

    let err = session
        .execute_source(
            "UNWIND [1, 2] AS x RETURN x AS g, count(*) AS c GROUP BY x",
            &EmptyProcedureRegistry,
        )
        .expect_err("two distinct group keys exceed the group-by cap of 1");

    assert!(
        matches!(
            err,
            ExecutorError::ProgramLimitExceeded {
                detail: "GROUP BY distinct-group cap exceeded",
                ..
            }
        ),
        "got {err:?}"
    );
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}
