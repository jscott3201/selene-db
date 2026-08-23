#![allow(missing_docs)]
//! Criterion benches for plan-corpus parsing throughput.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

fn bench_parse_corpus(c: &mut Criterion) {
    let entries = common::corpus_entries();
    let bytes = entries
        .iter()
        .map(|entry| entry.source.len())
        .sum::<usize>();
    let mut group = c.benchmark_group("gql_parse_corpus");
    group.throughput(Throughput::Bytes(bytes as u64));
    group.bench_function("m5c", |b| {
        b.iter(|| {
            for entry in &entries {
                std::hint::black_box(selene_gql::parse(entry.source).expect("source parses"));
            }
        });
    });
    group.finish();
}

fn bench_profile_conformance(c: &mut Criterion) {
    let feature_ids: Vec<_> = selene_profile::capabilities()
        .iter()
        .map(|record| record.id)
        .collect();
    let mut lookup = c.benchmark_group("gql_profile_conformance");
    lookup.throughput(Throughput::Elements(feature_ids.len() as u64));
    lookup.bench_function("generated_capability_lookup", |b| {
        b.iter(|| {
            for feature_id in &feature_ids {
                std::hint::black_box(
                    selene_profile::capability(*feature_id).expect("generated capability exists"),
                );
            }
        });
    });
    lookup.finish();

    let mut flagger = c.benchmark_group("gql_profile_flagger");
    flagger.throughput(Throughput::Elements(2));
    flagger.bench_function("parse_admitted_and_rejected", |b| {
        b.iter(|| {
            std::hint::black_box(
                selene_gql::parse("RETURN $parameter")
                    .expect("direct-selected GE04/GE05 parameter syntax is admitted"),
            );
            std::hint::black_box(
                selene_gql::parse("CREATE GRAPH demo")
                    .expect_err("implied unsupported GC04 is rejected"),
            );
        });
    });
    flagger.finish();
}

/// The six PARSER-DOS `[`-backtracking artifacts surfaced by the v1.1.0 fuzz
/// soak (mirrored from `tests/parser_dos_artifacts.rs`). Each is pure unclosed
/// `[`; all reject pre-pest via the `[`-depth complexity guard.
const BRACKET_ARTIFACTS: &[&[u8]] = &[
    b"lETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[CAs[ETQf.Enn55>[esETf[[[[CAsETQf[[esETf[[[[CAsETQf.Enn556esETf[[[CAs[ETQf.Enn55>[esETf[[[[CAsETQf[sETf[sETf[[[[CAsETQ[[esETf[[[[CAsETQf[sETf[[9[esETf[[[CAsETQf[sETf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETETQf[[esETf[[[[CAsETQf.Enn556esETf[[[CAs[ETQf.Enn55>[esETf[[[[CAsETQf[sETf[sETf[[[[CAsETQ[[esETf[[[[CAsETQf[sETf[[9[esETf[[[CAsETQf[sETf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[.<[005ETQ'\\LLL*-[5\xd8\xb10",
    b"lETl00000001=E.EnnTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CA0001=E.EnnTf[[[[CAsECAsETTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CA0001=E.EnnTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CAsETQQf[sETE.EQf[sETE.nnTf[[[[CAsECAsETQ[[[[[CA[[CAsECAsETQ[[CAsETQQf[sETE.EQf[sETE.ECETf[[[E000\x00\x00\x00\x00\x00\x00\x000.",
];

/// Near-cap zero-delimiter recursion inputs (the new BRIEF 01b crash class).
/// All sit just over `MAX_RECURSION_DEPTH = 256` so they reject pre-pest, but
/// far below the native-stack overflow floor so the bench never crashes.
fn recursion_artifacts() -> Vec<String> {
    let over_cap = 256 + 64;
    vec![
        format!("RETURN {}1", "-".repeat(over_cap)),
        format!("RETURN {}true", "NOT ".repeat(over_cap)),
        format!("RETURN {}1", "CASE WHEN true THEN ".repeat(over_cap)),
    ]
}

fn bench_parse_hostile(c: &mut Criterion) {
    let recursion = recursion_artifacts();
    let mut group = c.benchmark_group("gql_parse_hostile");
    group.bench_function("bracket_artifacts", |b| {
        b.iter(|| {
            for bytes in BRACKET_ARTIFACTS {
                let source = std::str::from_utf8(bytes).expect("artifact is valid UTF-8");
                // Rejected pre-pest; we only care it stays cheap and bounded.
                std::hint::black_box(selene_gql::parse(source).is_err());
            }
        });
    });
    group.bench_function("recursion_chains", |b| {
        b.iter(|| {
            for source in &recursion {
                std::hint::black_box(selene_gql::parse(source).is_err());
            }
        });
    });
    group.finish();
}

criterion_group! {
    name = parse_group;
    config = common::criterion_config();
    targets = bench_parse_corpus, bench_profile_conformance, bench_parse_hostile
}
criterion_main!(parse_group);
