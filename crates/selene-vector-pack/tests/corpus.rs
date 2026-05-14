//! Vector-pack mirror corpus tests.

use std::collections::BTreeSet;

use selene_core::{IStr, intern};
use selene_gql::{ProcedureMutability, ProcedureRegistry, ProcedureTier};
use selene_testing::VectorPackCorpus;
use selene_vector_pack::{VECTOR_PROCEDURE_NAMES, VectorPack};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

#[test]
fn vector_pack_corpus_renders_in_deterministic_order_and_covers_registry() {
    let corpus = VectorPackCorpus::b4_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        observed,
        VECTOR_PROCEDURE_NAMES.into_iter().collect::<BTreeSet<_>>()
    );
    insta::assert_snapshot!(corpus.render(), @r"
search_default [Search] CALL vector.search('default', [1.000000, 0.000000, 0.000000, 0.000000], 10, NULL, NULL)
upsert_default [Upsert] CALL vector.upsert('default', 42, [1.000000, 0.000000, 0.000000, 0.000000])
delete_default [Delete] CALL vector.delete('default', 42)
bulk_upsert_default [BulkUpsert] CALL vector.bulk_upsert('default', [42, 43], [[1.000000, 0.000000, 0.000000, 0.000000], [0.000000, 1.000000, 0.000000, 0.000000]])
bulk_delete_default [BulkDelete] CALL vector.bulk_delete('default', [42, 43])
ivf_bulk_upsert_default [IvfBulkUpsert] CALL vector.ivf_bulk_upsert('default', [42, 43], [[1.000000, 0.000000, 0.000000, 0.000000], [0.000000, 1.000000, 0.000000, 0.000000]])
ivf_bulk_delete_default [IvfBulkDelete] CALL vector.ivf_bulk_delete('default', [42, 43])
ivf_search_default [IvfSearch] CALL vector.ivf_search('default', [1.000000, 0.000000, 0.000000, 0.000000], 10, NULL, NULL)
ivf_search_n_probe_override [IvfSearch] CALL vector.ivf_search('default', [0.000000, 1.000000, 0.000000, 0.000000], 5, 2, NULL)
ivf_search_filtered [IvfSearch] CALL vector.ivf_search('default', [0.000000, 0.000000, 1.000000, 0.000000], 3, NULL, [42, 43])
ivf_stats_trained [IvfStats] CALL vector.ivf_stats('default')
ivf_stats_deferred [IvfStats] CALL vector.ivf_stats('default')
");
}

#[test]
fn vector_procedure_names_include_b4_entries() {
    assert_eq!(VECTOR_PROCEDURE_NAMES.len(), 9);
    assert!(VECTOR_PROCEDURE_NAMES.contains(&&["vector", "ivf_search"][..]));
    assert!(VECTOR_PROCEDURE_NAMES.contains(&&["vector", "ivf_stats"][..]));
}

#[test]
fn registry_exposes_three_graph_tier_vector_procedures() {
    let pack = VectorPack::new();
    let registry = pack
        .registry_with_builtins()
        .expect("vector pack registers cleanly");
    let mut graph_tier_count = 0;

    for name in VECTOR_PROCEDURE_NAMES {
        let interned = name.iter().map(|segment| istr(segment)).collect::<Vec<_>>();
        let metadata = registry.lookup(&interned).expect("procedure registered");
        if metadata.tier == ProcedureTier::Graph {
            graph_tier_count += 1;
            assert_eq!(metadata.mutability, ProcedureMutability::Read);
        }
    }

    assert_eq!(graph_tier_count, 3);
}

#[test]
fn b4_seed_covers_every_declared_procedure_name() {
    let observed = VectorPackCorpus::b4_seed()
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();

    for name in VECTOR_PROCEDURE_NAMES {
        assert!(observed.contains(name), "missing corpus entry for {name:?}");
    }
}
