//! Pipeline OrderBy executor tests.

mod exec_common;

use selene_core::Value;

use exec_common::{column_values, execute_read, node_ids_for};

#[test]
fn order_by_single_key_ascending() {
    let table = execute_read("FOR x IN [3, 1, 2] RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn order_by_single_key_descending() {
    let table = execute_read("FOR x IN [3, 1, 2] RETURN x ORDER BY x DESC");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(3), Value::Int(2), Value::Int(1)]
    );
}

#[test]
fn order_by_multi_key_with_first_key_ties_breaks_on_second() {
    let table = execute_read("MATCH (n:Person) RETURN * ORDER BY n.tenant ASC, n.score DESC");

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2), Some(3)]);
}

#[test]
fn order_by_is_stable_for_equal_keys() {
    let table = execute_read("MATCH (n:Person) RETURN * ORDER BY n.tenant");

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2), Some(3)]);
}

#[test]
fn order_by_nulls_last_under_ascending() {
    let table = execute_read("FOR x IN [2, NULL, 1] RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2), Value::Null]
    );
}

#[test]
fn order_by_nulls_first_under_descending() {
    let table = execute_read("FOR x IN [2, NULL, 1] RETURN x ORDER BY x DESC");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Null, Value::Int(2), Value::Int(1)]
    );
}

#[test]
fn order_by_explicit_nulls_policy_is_direction_aware_across_all_four_combos() {
    // GQLRT-28: only the two DEFAULT null placements (ASC->last, DESC->first)
    // were tested. The explicit `NULLS FIRST/LAST` × `ASC/DESC` matrix exercises
    // the direction-aware flip in `order_by::null_sort_order` (under DESC the
    // comparator reverses, so the policy must be flipped to land NULLs where the
    // user asked). Assert the full row order for all four combinations.
    let source = "FOR x IN [2, NULL, 1] RETURN x ORDER BY x";

    // ASC: data ascends 1,2; NULLS FIRST puts NULL at the head, LAST at the tail.
    assert_eq!(
        column_values(&execute_read(&format!("{source} ASC NULLS FIRST")), "x"),
        vec![Value::Null, Value::Int(1), Value::Int(2)]
    );
    assert_eq!(
        column_values(&execute_read(&format!("{source} ASC NULLS LAST")), "x"),
        vec![Value::Int(1), Value::Int(2), Value::Null]
    );

    // DESC: data descends 2,1; the explicit policy still honors FIRST/LAST
    // literally despite the reversed comparator.
    assert_eq!(
        column_values(&execute_read(&format!("{source} DESC NULLS FIRST")), "x"),
        vec![Value::Null, Value::Int(2), Value::Int(1)]
    );
    assert_eq!(
        column_values(&execute_read(&format!("{source} DESC NULLS LAST")), "x"),
        vec![Value::Int(2), Value::Int(1), Value::Null]
    );
}

#[test]
fn order_by_list_values_sort_lexicographically() {
    // ISO §22.14 ordering: lists order element-wise by ascending ordinal; the
    // first differing element decides ([1,2] < [1,3] < [2,0]).
    let table = execute_read("FOR x IN [[2, 0], [1, 3], [1, 2]] RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(1), Value::Int(3)]),
            Value::List(vec![Value::Int(2), Value::Int(0)]),
        ]
    );
}

#[test]
fn order_by_list_values_shorter_prefix_precedes() {
    // Cardinality tiebreak: on an equal prefix the shorter list sorts first.
    let table = execute_read("FOR x IN [[1, 2, 3], [1, 2]] RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        ]
    );
}

#[test]
fn order_by_list_values_descending_reverses_lexicographic_order() {
    let table = execute_read("FOR x IN [[1, 2], [2, 0], [1, 3]] RETURN x ORDER BY x DESC");

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::List(vec![Value::Int(2), Value::Int(0)]),
            Value::List(vec![Value::Int(1), Value::Int(3)]),
            Value::List(vec![Value::Int(1), Value::Int(2)]),
        ]
    );
}

// ---------------------------------------------------------------------------
// ORDER BY over a binding the RETURN discards (ISO §14.10, Features GA07/GQ14).
//
// The :Person fixture inserts scores 7, 3, 9 — deliberately neither ascending
// nor descending, so a sort that never runs is visible rather than accidentally
// correct. Insertion order is Alice, Bob, Cara.
// ---------------------------------------------------------------------------

fn names(table: &selene_gql::BindingTable) -> Vec<String> {
    column_values(table, "name")
        .into_iter()
        .map(|value| match value {
            Value::String(text) => text.as_str().to_owned(),
            other => panic!("expected a string name, got {other:?}"),
        })
        .collect()
}

/// The reported defect. `ORDER BY n.score` planned, reached the executor, and
/// did nothing: post-projection the binding `n` was gone, every row's key
/// evaluated to NULL, and the stable sort preserved insertion order.
#[test]
fn order_by_property_of_a_discarded_binding_sorts_descending() {
    let table = execute_read("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC");
    assert_eq!(names(&table), ["Cara", "Alice", "Bob"]);
}

#[test]
fn order_by_property_of_a_discarded_binding_sorts_ascending() {
    let table = execute_read("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score ASC");
    assert_eq!(names(&table), ["Bob", "Alice", "Cara"]);
}

/// The worst shape of the defect: one row came back, and it was the wrong one.
#[test]
fn order_by_property_of_a_discarded_binding_with_limit_one_returns_the_top_row() {
    let table =
        execute_read("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC LIMIT 1");
    assert_eq!(names(&table), ["Cara"]);
}

/// The carrier is the referenced binding, not the sort expression, so an
/// expression around the property needs no separate handling.
#[test]
fn order_by_expression_over_a_discarded_binding_sorts() {
    let table = execute_read("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score + 0 DESC");
    assert_eq!(names(&table), ["Cara", "Alice", "Bob"]);
}

/// Sorting on a string property, which the issue reported separately.
#[test]
fn order_by_string_property_of_a_discarded_binding_sorts() {
    let table = execute_read("MATCH (n:Person) RETURN n.score AS score ORDER BY n.name DESC");
    assert_eq!(
        column_values(&table, "score"),
        vec![Value::Int(9), Value::Int(3), Value::Int(7)]
    );
}

/// The carrier must not reach the caller. ISO §14.10 GR 1)b)ii drops exactly
/// the appended columns once the ordering statement has run.
#[test]
fn sort_carrier_columns_are_not_visible_in_the_result() {
    let table = execute_read("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC");
    let columns = table
        .schema()
        .columns
        .iter()
        .map(|column| column.name.clone().map(|name| name.as_str().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(columns, vec![Some("name".to_owned())]);
    for row in table.rows() {
        assert_eq!(row.values().len(), 1, "carrier survived into a result row");
    }
}

/// The carrier is only emitted when it is actually needed: a plan whose sort
/// keys all resolve to projected columns carries nothing and trims nothing.
///
/// This is what the alias-shadowing check in `carrier_projects` buys. It is not
/// load-bearing for correctness — a duplicate column resolves to the first
/// match, which is always the projection — so a plan-shape assertion is the only
/// thing that can pin it.
#[test]
fn no_carrier_is_emitted_when_every_sort_key_is_a_projected_column() {
    for source in [
        // The alias shadows the incoming binding `n`.
        "MATCH (n:Person) RETURN n.name AS n ORDER BY n DESC",
        // An ordinary alias sort, which never needed a carrier.
        "MATCH (n:Person) RETURN n.score AS s ORDER BY s DESC",
    ] {
        let plan = exec_common::planned(source);
        assert!(
            !plan
                .pipeline
                .iter()
                .any(|op| matches!(op, selene_gql::PipelineOp::TrimOrderCarriers { .. })),
            "{source} needs no sort carrier, so it must not emit a trim"
        );
    }
}

/// ...and the converse: a discarded binding does emit one.
#[test]
fn a_carrier_is_emitted_when_a_sort_key_reaches_past_the_projection() {
    let plan = exec_common::planned("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC");
    let trims = plan
        .pipeline
        .iter()
        .filter(|op| matches!(op, selene_gql::PipelineOp::TrimOrderCarriers { .. }))
        .count();
    assert_eq!(trims, 1, "exactly one trim closes the carrier");
    assert_eq!(
        plan.output_schema.columns.len(),
        1,
        "the carrier must not widen the declared output schema"
    );
}

/// A return alias beats an incoming column of the same name: ISO SR VIII
/// appends only for references the return identifiers do not already cover.
#[test]
fn a_return_alias_shadows_an_incoming_binding_of_the_same_name() {
    let table = execute_read("MATCH (n:Person) RETURN n.name AS n ORDER BY n DESC");
    let sorted = column_values(&table, "n")
        .into_iter()
        .map(|value| match value {
            Value::String(text) => text.as_str().to_owned(),
            other => panic!("expected a string, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(sorted, ["Cara", "Bob", "Alice"]);
}

// ---------------------------------------------------------------------------
// ORDER_REFS under GROUP BY (ISO §14.10 SR 4)c)i)2)A)III case 2).
//
// Case 2 is unconditional on DISTINCT: with a GROUP BY present, ORDER_REFS is
// the return identifiers plus every binding-variable reference in the grouping
// clause. SR VIII then carries exactly that difference across the projection.
// ---------------------------------------------------------------------------

/// `[3, 1, 2, 1]` groups to `3→1, 2→1, 1→2`, so the counts alone reveal the
/// order: an unsorted result is `[1, 2, 1]` in group-encounter order.
#[test]
fn group_by_orders_by_a_grouping_key_the_return_discards() {
    let table = execute_read("FOR x IN [3, 1, 2, 1] RETURN count(*) AS c GROUP BY x ORDER BY x");
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(2), Value::Int(1), Value::Int(1)]
    );
}

#[test]
fn group_by_orders_by_a_grouping_key_the_return_discards_descending() {
    let table =
        execute_read("FOR x IN [3, 1, 2, 1] RETURN count(*) AS c GROUP BY x ORDER BY x DESC");
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(1), Value::Int(1), Value::Int(2)]
    );
}

/// The grouping key is a property access, so the carried reference is the node
/// `n` rather than the key expression.
#[test]
fn group_by_orders_by_a_grouping_key_expression() {
    let table = execute_read(
        "MATCH (n:Person) RETURN n.tenant AS t, count(*) AS c \
         GROUP BY n.tenant ORDER BY n.tenant DESC",
    );
    assert_eq!(
        column_values(&table, "t"),
        vec![
            Value::String(exec_common::db_string("t2")),
            Value::String(exec_common::db_string("t1")),
        ]
    );
    assert_eq!(
        table.schema().columns.len(),
        2,
        "the carried grouping binding must not reach the caller"
    );
}

/// Ordering by an alias the projection keeps still emits no carrier, so the
/// pre-existing GROUP BY plans are untouched.
#[test]
fn group_by_ordering_by_a_projected_alias_is_unchanged() {
    let table =
        execute_read("FOR x IN [1, 2, 1] RETURN x AS x, count(*) AS c GROUP BY x ORDER BY x");
    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2)]
    );
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(2), Value::Int(1)]
    );
}

/// SR IX replaces the return item list with the *augmented* list before the set
/// quantifier applies, so a carrier widens the `DISTINCT` key. Under GROUP BY
/// the grouping keys are already unique per group, which makes the dedup a
/// no-op — `[3, 1, 2, 1]` keeps all three rows even though two share `c = 1`.
#[test]
fn a_carrier_participates_in_distinct_per_sr_ix() {
    let table =
        execute_read("FOR x IN [3, 1, 2, 1] RETURN DISTINCT count(*) AS c GROUP BY x ORDER BY x");
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(2), Value::Int(1), Value::Int(1)]
    );
}

/// `RETURN *` emits no projection, so nothing is discarded and no carrier is
/// needed — including under `DISTINCT`, which is what the star early-return in
/// `order_refs` buys. Scores are Alice 7, Bob 3, Cara 9.
#[test]
fn return_distinct_star_orders_by_an_incoming_binding() {
    let table = execute_read("MATCH (n:Person) RETURN DISTINCT * ORDER BY n.score");
    let ids = node_ids_for(&table, "n");
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids[0],
        node_ids_for(
            &execute_read("MATCH (n:Person) RETURN * ORDER BY n.score"),
            "n"
        )[0]
    );
}

// ---------------------------------------------------------------------------
// The carrier gate must not under-approximate the runtime schema.
//
// Both shapes below are ISO-legal, resolve at bind time, and are present in the
// runtime row before the projection — but were invisible to an earlier gate
// keyed off the planner's `visible` list, so they reached `ORDER BY` with the
// column already gone.
// ---------------------------------------------------------------------------

/// Path bindings live in the runtime pattern schema but not in the planner's
/// visible list.
#[test]
fn order_by_a_path_binding_resolves() {
    let table =
        execute_read("MATCH p = (a:Person)-[e:KNOWS]->(b:Person) RETURN b.name AS name ORDER BY p");
    assert_eq!(table.schema().columns.len(), 1);
}

/// A subquery body's imported outer bindings are appended by the runtime but
/// are not in the body's own pattern plan.
#[test]
fn call_subquery_body_orders_by_an_imported_outer_binding() {
    let table = execute_read(
        "MATCH (a:Person) CALL (a) { MATCH (d:Person) RETURN d.name AS n ORDER BY a.score LIMIT 1 } \
         YIELD n RETURN n ORDER BY n",
    );
    assert_eq!(table.row_count(), 3);
    assert_eq!(table.schema().columns.len(), 1);
}

// ---------------------------------------------------------------------------
// Guards that only a deliberately broken plan can exercise.
// ---------------------------------------------------------------------------

/// The strict miss path in `lookup_variable`. Carriers keep every legal plan
/// supplied and the analyzer rejects the rest, so the only way to reach this is
/// a plan whose schema genuinely disagrees with its sort key — which is exactly
/// what the guard is for. Deleting the carrier from a planned query reproduces
/// the original defect's plan shape.
#[test]
fn order_by_over_a_column_the_projection_dropped_is_a_hard_error() {
    let mut plan =
        exec_common::planned("MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC");
    for op in &mut plan.pipeline {
        if let selene_gql::PipelineOp::Project(items) = op {
            assert_eq!(
                items.len(),
                2,
                "expected one projected column plus a carrier"
            );
            items.truncate(1);
        }
    }
    plan.pipeline
        .retain(|op| !matches!(op, selene_gql::PipelineOp::TrimOrderCarriers { .. }));

    let fixture = exec_common::ExecFixture::build();
    let error = exec_common::execute_plan(&fixture, &plan)
        .expect_err("a sort key with no column must fail loudly, not sort by NULL");
    assert!(
        matches!(&error, selene_gql::ExecutorError::InvalidReference { name, .. } if name == "n"),
        "expected InvalidReference for `n`, got {error:?}"
    );
}

/// The trim must sit after the sort so `OrderBy` and `Limit` stay adjacent for
/// the TopK rule; emitting it between them silently degrades a bounded sort to
/// a full sort while every result assertion stays green.
#[test]
fn the_optimized_plan_fuses_top_k_and_still_trims_the_carrier() {
    let table = exec_common::execute_optimized_read(
        "MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC LIMIT 1",
    );
    assert_eq!(names(&table), ["Cara"]);
    assert_eq!(table.schema().columns.len(), 1, "carrier survived the trim");

    let fixture = exec_common::ExecFixture::build();
    let plan = exec_common::optimized(
        "MATCH (n:Person) RETURN n.name AS name ORDER BY n.score DESC LIMIT 1",
        &fixture.index_catalog(),
    );
    let shape = plan
        .pipeline
        .iter()
        .map(|op| match op {
            selene_gql::PipelineOp::TopK { .. } => "TopK",
            selene_gql::PipelineOp::OrderBy(_) => "OrderBy",
            selene_gql::PipelineOp::Limit { .. } => "Limit",
            selene_gql::PipelineOp::TrimOrderCarriers { .. } => "Trim",
            selene_gql::PipelineOp::Project(_) => "Project",
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shape,
        vec!["Project", "TopK", "Trim"],
        "the trim must follow the fused TopK, not separate OrderBy from Limit"
    );
}

/// ISO §14.10 SR IV/VIII reach into an `EXISTS` body. §5.3.2.1 defines "contain"
/// transitively, so the free `n` inside the subquery is a binding variable
/// reference contained in the sort key, and SR VIII has to carry it past the
/// projection. Without the carrier the runtime cannot find `n` in the
/// post-projection row and raises an internal-invariant error for a query the
/// analyzer accepted.
///
/// Fixture: Alice-KNOWS->Bob and Bob-KNOWS->Sensor, Cara has no outgoing edge.
#[test]
fn an_exists_sort_key_carries_its_free_outer_reference() {
    let table = execute_read(
        "MATCH (n:Person) RETURN n.name AS name \
         ORDER BY EXISTS { MATCH (n)-[:KNOWS]->() } DESC, n.name ASC",
    );
    let names = column_values(&table, "name")
        .into_iter()
        .map(|value| match value {
            Value::String(text) => text.as_str().to_owned(),
            other => panic!("expected a string name, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()],
        "the two people with outgoing KNOWS sort ahead of the one without"
    );
}

/// Carriers appended past the projected width, per ISO §14.10 SR VIII, which
/// GR 1)b)ii then drops again.
fn carrier_count(plan: &selene_gql::ExecutionPlan) -> usize {
    let Some(projected) = plan.pipeline.iter().find_map(|op| match op {
        selene_gql::PipelineOp::TrimOrderCarriers { projected_width } => Some(*projected_width),
        _ => None,
    }) else {
        return 0;
    };
    let total = plan
        .pipeline
        .iter()
        .find_map(|op| match op {
            selene_gql::PipelineOp::Project(projects) => Some(projects.len()),
            _ => None,
        })
        .expect("a trim implies a project");
    total - projected
}

#[test]
fn an_exists_sort_key_emits_exactly_one_carrier_and_does_not_widen_the_schema() {
    let plan = exec_common::planned(
        "MATCH (n:Person) RETURN n.name AS name ORDER BY EXISTS { MATCH (n)-[:KNOWS]->() }",
    );
    assert_eq!(
        carrier_count(&plan),
        1,
        "only the free outer `n` is carried"
    );
    assert_eq!(
        plan.pipeline
            .iter()
            .filter(|op| matches!(op, selene_gql::PipelineOp::TrimOrderCarriers { .. }))
            .count(),
        1,
        "exactly one trim closes the carrier"
    );
    assert_eq!(
        plan.output_schema.columns.len(),
        1,
        "the carrier must not widen the declared output schema"
    );
}

/// The subtraction that matters: a variable the subquery pattern binds itself is
/// not a reference to anything outside, so it gets no carrier. Presence-only
/// collection would carry `m` here and widen the row for nothing.
#[test]
fn a_variable_bound_inside_the_exists_body_gets_no_carrier() {
    // The body must *reference* its own variable, not merely declare it. A bare
    // `(m:Person)` is a declaration and produces no reference at all, so it
    // cannot tell a correct subtraction from a missing one; the `WHERE` is what
    // makes `m` a reference whose declaration sits inside the sort key.
    let plan = exec_common::planned(
        "MATCH (n:Person) RETURN n.name AS name \
         ORDER BY EXISTS { MATCH (n)-[:KNOWS]->(m:Person) WHERE m.age > 1 }",
    );
    assert_eq!(
        carrier_count(&plan),
        1,
        "`n` is carried and `m` is not: m is defined inside the sort key, so it \
         is not a reference to a discarded binding (ISO §14.10 CR 4)"
    );

    // And the same shape with no outer reference at all carries nothing.
    let plan = exec_common::planned(
        "MATCH (n:Person) RETURN n.name AS name \
         ORDER BY EXISTS { MATCH (m:Person) WHERE m.age > 1 }",
    );
    assert_eq!(carrier_count(&plan), 0, "m alone is never carried");
}

/// An uncorrelated EXISTS reaches nothing outside itself, so it needs no carrier
/// at all and must not emit a trim.
#[test]
fn an_uncorrelated_exists_sort_key_emits_no_carrier() {
    let plan = exec_common::planned(
        "MATCH (n:Person) RETURN n.name AS name ORDER BY EXISTS { MATCH (x:Person) }",
    );
    assert_eq!(carrier_count(&plan), 0);
    assert!(
        !plan
            .pipeline
            .iter()
            .any(|op| matches!(op, selene_gql::PipelineOp::TrimOrderCarriers { .. })),
        "nothing to trim when nothing was carried"
    );
}

/// The regression guard for the symptom #1112 reported: an analyzer-accepted
/// sort key must not reach the user as an internal-invariant diagnostic.
#[test]
fn an_accepted_exists_sort_key_never_surfaces_an_implementation_defined_error() {
    for source in [
        "MATCH (n:Person) RETURN n.name AS name ORDER BY EXISTS { MATCH (n)-[:KNOWS]->() }",
        "MATCH (n:Person) RETURN n.name AS name \
         ORDER BY EXISTS { MATCH (n)-[:KNOWS]->(m:Person) } DESC, n.name",
        "MATCH (n:Person) RETURN n.name AS name, n.score AS score \
         ORDER BY EXISTS { MATCH (n)-[:KNOWS]->() }, score",
    ] {
        let table = execute_read(source);
        assert_eq!(table.rows().len(), 3, "{source} returns every Person");
    }
}
