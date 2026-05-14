//! M11 corpus-harness closeout snapshot battery.

use std::collections::BTreeSet;
use std::path::Path;

use selene_testing::{VectorPackCorpus, VectorPackCorpusCategory, VectorPackCorpusEntry};
use selene_vector_pack::VECTOR_PROCEDURE_NAMES;

#[test]
fn corpus_snapshots_match() {
    let corpus = VectorPackCorpus::b4_seed();
    for entry in corpus.entries() {
        insta::with_settings!({ snapshot_suffix => entry.name }, {
            insta::assert_snapshot!(entry.render());
        });
    }
}

#[test]
fn corpus_slugs_are_unique() {
    let corpus = VectorPackCorpus::b4_seed();
    let mut seen = BTreeSet::new();
    for entry in corpus.entries() {
        assert!(seen.insert(entry.name), "duplicate slug: {}", entry.name);
    }
    assert_eq!(seen.len(), 12);
}

#[test]
fn corpus_categories_covered() {
    let corpus = VectorPackCorpus::b4_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.category)
        .collect::<BTreeSet<_>>();
    let declared = VectorPackCorpusCategory::ALL
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    assert_eq!(observed, declared);
}

#[test]
fn corpus_covers_every_procedure() {
    let corpus = VectorPackCorpus::b4_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<BTreeSet<_>>();
    let declared = VECTOR_PROCEDURE_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    assert_eq!(observed, declared);
}

#[test]
fn procedure_name_accessor_matches_render_call_token() {
    let corpus = VectorPackCorpus::b4_seed();
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
fn every_b_seed_constructor_preserves_post_b4_invariants() {
    let seeds = [
        VectorPackCorpus::b1_seed(),
        VectorPackCorpus::b2_seed(),
        VectorPackCorpus::b3_seed(),
        VectorPackCorpus::b4_seed(),
    ];
    for pair in seeds.windows(2) {
        assert_ordered_subsequence(pair[0].entries(), pair[1].entries());
    }

    let first_occurrence_order =
        first_occurrence_procedure_order(seeds.last().expect("b4 seed exists").entries());
    assert_eq!(first_occurrence_order, VECTOR_PROCEDURE_NAMES);
}

#[test]
fn vector_pack_bench_registered_in_runner() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-benches.sh"),
    )
    .expect("read run-benches.sh");

    assert!(
        script.contains("selene-vector-pack:vector_pack:criterion"),
        "scripts/run-benches.sh BENCHES list is missing \
         selene-vector-pack:vector_pack:criterion"
    );
}

fn assert_ordered_subsequence(previous: &[VectorPackCorpusEntry], next: &[VectorPackCorpusEntry]) {
    let mut cursor = next.iter();
    for entry in previous {
        assert!(
            cursor.any(|candidate| candidate == entry),
            "entry {entry:?} was removed or reordered"
        );
    }
}

fn first_occurrence_procedure_order(
    entries: &[VectorPackCorpusEntry],
) -> Vec<&'static [&'static str]> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for entry in entries {
        let name = entry.invocation.procedure_name();
        if seen.insert(name) {
            order.push(name);
        }
    }
    order
}
