//! Bench-runner registration drift checks.

use std::path::Path;

#[test]
fn bench_registration_is_pinned() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-benches.sh"),
    )
    .expect("read run-benches.sh");

    // #227 replaced the old colon-delimited `BENCHES="<crate>:<bench>:<layer>"`
    // list with the pipe-delimited `REGISTRY="<crate>|<bench>|<needs_test_harness>"`
    // block (the iai layer was dropped). Pin the current token so this drift
    // check stays meaningful — keep it in lockstep with the runner format and
    // its sibling pin `selene-testing/tests/benchmarks_md_pin.rs`.
    assert!(
        script.contains("selene-db-algorithms|algo_bench|"),
        "scripts/run-benches.sh REGISTRY is missing selene-db-algorithms|algo_bench|<harness>"
    );
}

#[test]
fn algo_bench_parallel_modes_are_pinned() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/algo_bench.rs"),
    )
    .expect("read algo_bench.rs");

    assert!(
        source.contains("const PARALLELISM_BENCH_MODES: &[(&str, Parallelism)]"),
        "BRIEF-82 introduced PARALLELISM_BENCH_MODES as the canonical \
         Sequential vs Auto bench mode table"
    );
    assert!(
        source.contains("Parallelism::Sequential"),
        "PARALLELISM_BENCH_MODES must keep the Sequential baseline"
    );
    assert!(
        source.contains("Parallelism::Auto"),
        "PARALLELISM_BENCH_MODES must keep the Auto parallel baseline"
    );

    for fn_name in [
        "bench_pagerank",
        "bench_betweenness",
        "bench_triangle_count",
        "bench_apsp",
    ] {
        let body = slice_fn_body(&source, fn_name);
        assert!(
            body.contains("PARALLELISM_BENCH_MODES"),
            "BRIEF-82/83/84/85 require {fn_name} to measure both \
             Sequential and Auto via PARALLELISM_BENCH_MODES"
        );
    }

    let louvain = slice_fn_body(&source, "bench_louvain");
    assert!(
        !louvain.contains("PARALLELISM_BENCH_MODES"),
        "BRIEF-86 pins bench_louvain as sequential-only for v1.0"
    );
}

fn slice_fn_body<'a>(source: &'a str, fn_name: &str) -> &'a str {
    let start = source
        .find(&format!("fn {fn_name}("))
        .unwrap_or_else(|| panic!("{fn_name} not found in algo_bench.rs"));
    let tail = &source[start..];
    let next_fn_offset = tail[1..]
        .find("\nfn ")
        .map(|offset| offset + 1)
        .unwrap_or(tail.len());
    &tail[..next_fn_offset]
}
