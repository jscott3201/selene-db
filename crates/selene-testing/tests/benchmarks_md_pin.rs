//! Pins BENCHMARKS.md coverage against the benchmark runner registry.

use std::path::Path;

#[test]
fn benchmarks_md_lists_all_registered_benches() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script_path = manifest_dir.join("../../scripts/run-benches.sh");
    let benchmarks_path = manifest_dir.join("../../BENCHMARKS.md");
    let script = std::fs::read_to_string(&script_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", script_path.display()));
    let benchmarks = std::fs::read_to_string(&benchmarks_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", benchmarks_path.display()));

    let tokens = bench_tokens(&script);
    assert!(
        !tokens.is_empty(),
        "expected at least one crate:bench:layer token in scripts/run-benches.sh"
    );
    for token in tokens {
        assert!(
            benchmarks.contains(token),
            "BENCHMARKS.md is missing registered bench token `{token}`"
        );
    }
    assert!(
        benchmarks.contains("Hardware footprint"),
        "BENCHMARKS.md must include hardware capture instructions"
    );
    assert!(
        benchmarks.contains("iai-callgrind"),
        "BENCHMARKS.md must include the deferred iai-callgrind section"
    );
}

fn bench_tokens(script: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut in_benches = false;
    for line in script.lines() {
        if line.trim() == "BENCHES=\"" {
            in_benches = true;
            continue;
        }
        if in_benches && line.trim() == "\"" {
            break;
        }
        if in_benches {
            let candidate = line.trim();
            if is_bench_token(candidate) {
                tokens.push(candidate);
            }
        }
    }
    tokens
}

fn is_bench_token(candidate: &str) -> bool {
    let mut parts = candidate.split(':');
    let Some(crate_name) = parts.next() else {
        return false;
    };
    let Some(bench_name) = parts.next() else {
        return false;
    };
    let Some(layer) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [crate_name, bench_name, layer]
        .iter()
        .all(|part| is_token_part(part))
}

fn is_token_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
