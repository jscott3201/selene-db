//! M10 corpus-harness closeout snapshot battery.

use std::collections::BTreeSet;

use selene_algorithms_pack::ALGO_PROCEDURE_NAMES;
use selene_testing::{AlgoPackCorpus, AlgoPackCorpusCategory, AlgoPackCorpusEntry};

#[test]
fn corpus_snapshots_match() {
    let corpus = AlgoPackCorpus::b5_seed();
    for entry in corpus.entries() {
        insta::with_settings!({ snapshot_suffix => entry.name }, {
            insta::assert_snapshot!(entry.render());
        });
    }
}

#[test]
fn corpus_slugs_are_unique() {
    let corpus = AlgoPackCorpus::b5_seed();
    let mut seen = BTreeSet::new();
    for entry in corpus.entries() {
        assert!(seen.insert(entry.name), "duplicate slug: {}", entry.name);
    }
}

#[test]
fn corpus_categories_covered() {
    let corpus = AlgoPackCorpus::b5_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.category)
        .collect::<BTreeSet<_>>();
    let declared = AlgoPackCorpusCategory::ALL
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    assert_eq!(observed, declared);
}

#[test]
fn corpus_covers_every_procedure() {
    let corpus = AlgoPackCorpus::b5_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();
    let declared = ALGO_PROCEDURE_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    assert_eq!(observed, declared);
}

#[test]
fn procedure_name_accessor_matches_render_call_token() {
    let corpus = AlgoPackCorpus::b5_seed();
    for entry in corpus.entries() {
        let token = format!("CALL {}(", entry.invocation.procedure_name().join("."));
        assert!(
            entry.invocation.render_call().contains(&token),
            "{} render did not contain {token:?}: {}",
            entry.name,
            entry.invocation.render_call()
        );
    }
}

#[test]
fn every_b_seed_constructor_preserves_post_b5_invariants() {
    let seeds = [
        AlgoPackCorpus::b1_seed(),
        AlgoPackCorpus::b2_seed(),
        AlgoPackCorpus::b3_seed(),
        AlgoPackCorpus::b4_seed(),
        AlgoPackCorpus::b5_seed(),
    ];
    for pair in seeds.windows(2) {
        assert_ordered_subsequence(pair[0].entries(), pair[1].entries());
    }

    let b5_order = seeds
        .last()
        .expect("b5 seed exists")
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<Vec<_>>();
    assert_eq!(b5_order, ALGO_PROCEDURE_NAMES);
}

fn assert_ordered_subsequence(previous: &[AlgoPackCorpusEntry], next: &[AlgoPackCorpusEntry]) {
    let mut cursor = next.iter();
    for entry in previous {
        assert!(
            cursor.any(|candidate| candidate == entry),
            "entry {entry:?} was removed or reordered"
        );
    }
}
