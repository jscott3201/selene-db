//! Bench-runner registration drift checks.

use std::path::Path;

#[test]
fn bench_registration_is_pinned() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-benches.sh"),
    )
    .expect("read run-benches.sh");

    assert!(
        script.contains("selene-algorithms:algo_bench:criterion"),
        "scripts/run-benches.sh BENCHES list is missing \
         selene-algorithms:algo_bench:criterion"
    );
}
