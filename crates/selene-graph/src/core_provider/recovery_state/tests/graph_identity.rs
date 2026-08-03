//! Caller-asserted graph identity is checked against the WAL, not just the
//! snapshot.
//!
//! `CORE/META` has always been checked, so a snapshot-bearing directory could
//! not be opened under the wrong `GraphId`. A WAL-only directory has no
//! `CORE/META`, and before this suite nothing on disk contradicted a wrong
//! assertion: recovery rebuilt another graph's rows under the caller's name and
//! then kept writing into them.
//!
//! The evidence is the graph id production DDL stamps into every
//! `Change::SchemaChanged`. That makes the coverage partial by construction,
//! which `data_only_wal_declares_no_identity` pins deliberately rather than
//! leaving as an untested gap.

use selene_core::{
    Change, GraphId, LabelSet, NodeId, PropertyMap, SchemaChange, SchemaPropertyIndexKind,
    db_string,
};
use selene_persist::RecoveryProvider;

use crate::core_provider::sections::encode_meta;
use crate::core_provider::{CORE_META_SUB, CoreProvider};
use crate::{GraphError, IndexProvider, ProviderError, SharedGraph, SubTag};

const AUTHOR: GraphId = GraphId::new(7);
const IMPOSTOR: GraphId = GraphId::new(9);

/// A schema change an *open* graph can replay, so a refusal is attributable to
/// identity rather than to catalog DDL landing on an unbound graph.
fn schema_change(graph: GraphId) -> Change {
    Change::SchemaChanged {
        graph,
        change: SchemaChange::PropertyIndexCreated {
            label: db_string("Reading").unwrap(),
            property: db_string("celsius").unwrap(),
            kind: SchemaPropertyIndexKind::I64,
        },
    }
}

fn node_created() -> Change {
    Change::NodeCreated {
        id: NodeId::new(1),
        labels: LabelSet::single(db_string("Reading").unwrap()),
        properties: PropertyMap::new(),
    }
}

fn replay(provider: &CoreProvider, changes: &[Change]) {
    for change in changes {
        RecoveryProvider::on_change(provider, change).unwrap();
    }
}

/// Seed `CORE/META` for `graph_id` so the directory is snapshot-bearing.
fn load_meta(provider: &CoreProvider, graph_id: GraphId) {
    let snapshot = SharedGraph::builder(graph_id)
        .build()
        .unwrap()
        .read()
        .as_ref()
        .clone();
    IndexProvider::read_section(
        provider,
        SubTag(CORE_META_SUB),
        &encode_meta(&snapshot.meta, 0).unwrap(),
    )
    .unwrap();
}

/// Assert the refusal names both identities *in their roles*.
///
/// Matching the two ids in order rather than independently is deliberate: both
/// always appear, so a pair of `contains` checks passes even when the message
/// has them swapped, and an operator reading "caller asserted" would be told
/// they typed the id they in fact read off disk.
fn assert_identity_refusal(error: &GraphError, observed: GraphId, asserted: GraphId) {
    let GraphError::Provider(ProviderError::Inconsistent { reason }) = error else {
        panic!("expected an Inconsistent provider error, got {error:?}");
    };
    let expected = format!("declare {observed} but caller asserted {asserted}");
    assert!(
        reason.contains(&expected),
        "refusal must report {observed} as on-disk and {asserted} as asserted: {reason}"
    );
}

/// The defect: a WAL-only directory recovered under an id nobody wrote it with.
#[test]
fn wal_only_schema_record_under_wrong_graph_id_is_refused() {
    let provider = CoreProvider::new_for_recovery();
    replay(provider.as_ref(), &[node_created(), schema_change(AUTHOR)]);

    let error = provider
        .finish_recovery(IMPOSTOR, None)
        .expect_err("WAL authored by AUTHOR must not recover as IMPOSTOR");

    assert_identity_refusal(&error, AUTHOR, IMPOSTOR);
}

#[test]
fn wal_only_schema_record_under_matching_graph_id_recovers() {
    let provider = CoreProvider::new_for_recovery();
    replay(provider.as_ref(), &[node_created(), schema_change(AUTHOR)]);

    let recovered = provider.finish_recovery(AUTHOR, None).unwrap();

    assert_eq!(recovered.meta.graph_id, AUTHOR);
    assert_eq!(recovered.property_index_count(), 1);
}

/// The honest limit of this check. Only schema records name their graph, so a
/// WAL of pure data changes declares no identity and any assertion is accepted.
/// Pinned so that tightening it later is a deliberate change to a stated
/// contract rather than an accident.
#[test]
fn data_only_wal_declares_no_identity() {
    let provider = CoreProvider::new_for_recovery();
    replay(provider.as_ref(), &[node_created()]);

    let recovered = provider
        .finish_recovery(IMPOSTOR, None)
        .expect("a data-only WAL carries nothing to contradict the caller");

    assert_eq!(recovered.meta.graph_id, IMPOSTOR);
}

/// A WAL holding two identities is cross-wired whichever id the caller asserts,
/// so recovering it under the first one still replays a second graph's commits.
#[test]
fn cross_wired_wal_is_refused_even_under_its_first_identity() {
    let provider = CoreProvider::new_for_recovery();
    replay(
        provider.as_ref(),
        &[schema_change(AUTHOR), schema_change(IMPOSTOR)],
    );

    let error = provider
        .finish_recovery(AUTHOR, None)
        .expect_err("a WAL carrying two identities must not recover under either");

    assert_identity_refusal(&error, IMPOSTOR, AUTHOR);
}

/// The mirror of the case above, and the one that pins *which* id is retained.
///
/// Recovering a `[AUTHOR, IMPOSTOR]` WAL under `AUTHOR` refuses under either a
/// first-wins or a last-wins rule, so that test alone does not distinguish
/// them. Under `IMPOSTOR` they diverge: last-wins finds the retained id equal
/// to the caller's, sees nothing to disagree with, and admits the graph — the
/// exact silent cross-wiring this check exists to stop.
#[test]
fn cross_wired_wal_is_refused_under_its_last_identity_too() {
    let provider = CoreProvider::new_for_recovery();
    replay(
        provider.as_ref(),
        &[schema_change(AUTHOR), schema_change(IMPOSTOR)],
    );

    let error = provider
        .finish_recovery(IMPOSTOR, None)
        .expect_err("the earlier identity in the WAL is still evidence against the caller");

    assert_identity_refusal(&error, AUTHOR, IMPOSTOR);
}

/// A `GraphReset` truncates and re-opens a graph; it does not rename one. The
/// pre-reset identity therefore stays binding, and the reset arm of
/// `into_graph` — which builds its meta straight from the caller's id — must be
/// gated too.
#[test]
fn graph_reset_does_not_clear_the_wal_identity() {
    let provider = CoreProvider::new_for_recovery();
    replay(
        provider.as_ref(),
        &[schema_change(AUTHOR), Change::GraphReset {}],
    );

    let error = provider
        .finish_recovery(IMPOSTOR, None)
        .expect_err("a reset moots schema intent, not the graph's identity");

    assert_identity_refusal(&error, AUTHOR, IMPOSTOR);
}

/// The snapshot arms already agreed with the caller, so a disagreeing WAL is
/// the same cross-wiring one layer up: another graph's commits appended onto
/// this graph's directory.
#[test]
fn snapshot_meta_matching_caller_does_not_excuse_a_foreign_wal() {
    let provider = CoreProvider::new_for_recovery();
    load_meta(provider.as_ref(), AUTHOR);
    replay(provider.as_ref(), &[schema_change(IMPOSTOR)]);

    let error = provider
        .finish_recovery(AUTHOR, None)
        .expect_err("CORE/META agreeing with the caller does not vouch for the WAL");

    assert_identity_refusal(&error, IMPOSTOR, AUTHOR);
}
