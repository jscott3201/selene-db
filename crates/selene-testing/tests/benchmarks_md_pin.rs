//! Registry↔doc parity lint: pins BENCHMARKS.md coverage against the runner.
//!
//! Every bench bin registered in `scripts/run-benches.sh` (the `REGISTRY`
//! block, `<crate>|<bench>|<needs_test_harness>`) MUST be named in
//! BENCHMARKS.md, so a newly added bench can never ship undocumented — the
//! historical `expression_eval` orphan. Mirrored as a fast-gate shell check in
//! `.github/scripts/check-benchmarks-doc.sh`; this test is the release-gate
//! enforcement under nextest.

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

    let benches = registry_benches(&script);
    assert!(
        !benches.is_empty(),
        "expected at least one <crate>|<bench>|<harness> row in the run-benches.sh REGISTRY block"
    );
    for (crate_name, bench) in &benches {
        assert!(
            benchmarks.contains(bench.as_str()),
            "BENCHMARKS.md is missing registered bench `{crate_name}::{bench}` \
             — every REGISTRY bench bin must be documented (no silent orphans)"
        );
    }
    assert!(
        benchmarks.contains("Hardware footprint"),
        "BENCHMARKS.md must include the hardware footprint capture instructions"
    );
}

/// Extract `(crate, bench)` pairs from the `REGISTRY="..."` block in
/// run-benches.sh. The separate `SMOKE` block (also pipe-delimited) is
/// deliberately not parsed — only the authoritative registry gates the doc.
fn registry_benches(script: &str) -> Vec<(String, String)> {
    let mut benches = Vec::new();
    let mut in_registry = false;
    for line in script.lines() {
        let trimmed = line.trim();
        if trimmed == "REGISTRY=\"" {
            in_registry = true;
            continue;
        }
        if in_registry && trimmed == "\"" {
            break;
        }
        if !in_registry {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('|').collect();
        if parts.len() == 3 && is_token(parts[0]) && is_token(parts[1]) {
            benches.push((parts[0].to_string(), parts[1].to_string()));
        }
    }
    benches
}

fn is_token(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
