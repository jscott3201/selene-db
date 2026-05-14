//! Integration tests for the vector-pack mirror corpus.

use std::collections::BTreeSet;

use selene_testing::{VectorPackCorpus, VectorPackCorpusCategory, VectorPackInvocation};

#[test]
fn procedure_name_returns_static_slice_for_search_variant() {
    let invocation = VectorPackInvocation::Search {
        index_name: "default",
        query: &[1.0],
        k: 1,
        ef_search: None,
        filter_nodes: &[],
    };

    assert_eq!(invocation.procedure_name(), &["vector", "search"]);
}

#[test]
fn procedure_name_returns_static_slice_for_upsert_variant() {
    let invocation = VectorPackInvocation::Upsert {
        index_name: "default",
        node_id: 1,
        vector: &[1.0],
    };

    assert_eq!(invocation.procedure_name(), &["vector", "upsert"]);
}

#[test]
fn procedure_name_returns_static_slice_for_delete_variant() {
    let invocation = VectorPackInvocation::Delete {
        index_name: "default",
        node_id: 1,
    };

    assert_eq!(invocation.procedure_name(), &["vector", "delete"]);
}

#[test]
fn procedure_name_returns_static_slice_for_bulk_variants() {
    let invocations = [
        (
            VectorPackInvocation::BulkUpsert {
                index_name: "default",
                node_ids: &[1],
                vectors: &[&[1.0]],
            },
            &["vector", "bulk_upsert"][..],
        ),
        (
            VectorPackInvocation::BulkDelete {
                index_name: "default",
                node_ids: &[1],
            },
            &["vector", "bulk_delete"][..],
        ),
        (
            VectorPackInvocation::IvfBulkUpsert {
                index_name: "default",
                node_ids: &[1],
                vectors: &[&[1.0]],
            },
            &["vector", "ivf_bulk_upsert"][..],
        ),
        (
            VectorPackInvocation::IvfBulkDelete {
                index_name: "default",
                node_ids: &[1],
            },
            &["vector", "ivf_bulk_delete"][..],
        ),
        (
            VectorPackInvocation::IvfSearch {
                index_name: "default",
                query: &[1.0],
                k: 1,
                n_probe: None,
                filter_nodes: &[],
            },
            &["vector", "ivf_search"][..],
        ),
        (
            VectorPackInvocation::IvfStats {
                index_name: "default",
            },
            &["vector", "ivf_stats"][..],
        ),
    ];

    for (invocation, expected) in invocations {
        assert_eq!(invocation.procedure_name(), expected);
    }
}

#[test]
fn procedure_name_covers_all_variants() {
    let invocations = [
        VectorPackInvocation::Search {
            index_name: "default",
            query: &[1.0],
            k: 1,
            ef_search: None,
            filter_nodes: &[],
        },
        VectorPackInvocation::Upsert {
            index_name: "default",
            node_id: 1,
            vector: &[1.0],
        },
        VectorPackInvocation::Delete {
            index_name: "default",
            node_id: 1,
        },
        VectorPackInvocation::BulkUpsert {
            index_name: "default",
            node_ids: &[1],
            vectors: &[&[1.0]],
        },
        VectorPackInvocation::BulkDelete {
            index_name: "default",
            node_ids: &[1],
        },
        VectorPackInvocation::IvfBulkUpsert {
            index_name: "default",
            node_ids: &[1],
            vectors: &[&[1.0]],
        },
        VectorPackInvocation::IvfBulkDelete {
            index_name: "default",
            node_ids: &[1],
        },
        VectorPackInvocation::IvfSearch {
            index_name: "default",
            query: &[1.0],
            k: 1,
            n_probe: None,
            filter_nodes: &[],
        },
        VectorPackInvocation::IvfStats {
            index_name: "default",
        },
    ];

    let mut names = BTreeSet::new();
    for invocation in invocations {
        let name = invocation.procedure_name();
        assert_eq!(name.len(), 2);
        assert_eq!(name[0], "vector");
        assert!(names.insert(name));
    }
    assert_eq!(names.len(), 9);
}

#[test]
fn category_all_matches_exhaustive_anchor() {
    let declared = VectorPackCorpusCategory::ALL
        .iter()
        .map(|category| category.stable_label())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        declared,
        [
            "Search",
            "Upsert",
            "Delete",
            "BulkUpsert",
            "BulkDelete",
            "IvfBulkUpsert",
            "IvfBulkDelete",
            "IvfSearch",
            "IvfStats",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
}

#[test]
fn entry_render_matches_corpus_render_line() {
    let corpus = VectorPackCorpus::b1_seed();
    let rendered_entries = corpus
        .entries()
        .iter()
        .map(selene_testing::VectorPackCorpusEntry::render)
        .collect::<String>();

    assert_eq!(corpus.render(), rendered_entries);
}

#[test]
fn corpus_b1_seed_unchanged_inside_b2_seed() {
    let b1 = VectorPackCorpus::b1_seed();
    let b2 = VectorPackCorpus::b2_seed();

    assert_eq!(&b2.entries()[..b1.entries().len()], b1.entries());
}

#[test]
fn corpus_b2_seed_covers_all_declared_procedures() {
    let names = VectorPackCorpus::b2_seed()
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        names,
        [
            &["vector", "search"][..],
            &["vector", "upsert"][..],
            &["vector", "delete"][..],
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
}

#[test]
fn corpus_b2_seed_unchanged_inside_b3_seed() {
    let b2 = VectorPackCorpus::b2_seed();
    let b3 = VectorPackCorpus::b3_seed();

    assert_eq!(&b3.entries()[..b2.entries().len()], b2.entries());
}

#[test]
fn corpus_b3_seed_covers_all_declared_procedures() {
    let names = VectorPackCorpus::b3_seed()
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        names,
        [
            &["vector", "search"][..],
            &["vector", "upsert"][..],
            &["vector", "delete"][..],
            &["vector", "bulk_upsert"][..],
            &["vector", "bulk_delete"][..],
            &["vector", "ivf_bulk_upsert"][..],
            &["vector", "ivf_bulk_delete"][..],
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
}

#[test]
fn corpus_b3_seed_unchanged_inside_b4_seed() {
    let b3 = VectorPackCorpus::b3_seed();
    let b4 = VectorPackCorpus::b4_seed();

    assert_eq!(&b4.entries()[..b3.entries().len()], b3.entries());
}

#[test]
fn corpus_b4_seed_covers_all_declared_procedures() {
    let names = VectorPackCorpus::b4_seed()
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        names,
        [
            &["vector", "search"][..],
            &["vector", "upsert"][..],
            &["vector", "delete"][..],
            &["vector", "bulk_upsert"][..],
            &["vector", "bulk_delete"][..],
            &["vector", "ivf_bulk_upsert"][..],
            &["vector", "ivf_bulk_delete"][..],
            &["vector", "ivf_search"][..],
            &["vector", "ivf_stats"][..],
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
}
