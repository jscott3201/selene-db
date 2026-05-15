//! Shared parallelism parser drift checks.

use std::path::Path;

#[test]
fn shared_parse_parallelism_is_canonical() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let canonical = std::fs::read_to_string(manifest_dir.join("src/parallel.rs"))
        .expect("read src/parallel.rs");

    assert_eq!(
        canonical.matches("fn parse_parallelism(").count(),
        1,
        "BRIEF-85 extracted crate::parallel::parse_parallelism as the \
         single canonical adapter parser"
    );

    for adapter in [
        "src/pathfinding.rs",
        "src/community.rs",
        "src/betweenness.rs",
        "src/pagerank.rs",
    ] {
        let source = std::fs::read_to_string(manifest_dir.join(adapter))
            .unwrap_or_else(|error| panic!("read {adapter}: {error}"));

        assert!(
            source.contains("parallel::parse_parallelism")
                || source.contains("use crate::parallel"),
            "{adapter} must import the shared crate::parallel::parse_parallelism helper"
        );
        assert!(
            !source.contains("fn parse_parallelism("),
            "{adapter} must not reintroduce an inline parse_parallelism duplicate"
        );
    }
}
