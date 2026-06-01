#![allow(missing_docs)]
#![allow(dead_code)]
//! Shared helpers for the `selene-algorithms` criterion benches.
//!
//! Mirrors the `common.rs` convention already used by the `selene-graph` and
//! `selene-persist` bench suites so the projection-build / neighbor-iter bench
//! and the algorithm baselines share one criterion config and one projection
//! builder (no per-bin drift).

use std::time::Duration;

use criterion::Criterion;
use selene_algorithms::{GraphProjection, ProjectionConfig};
use selene_graph::SeleneGraph;
use selene_testing::BenchProfile;

/// Profile-aware criterion config shared by every algorithms bench
/// (`quick` => sample 10 / 500ms; `full`/`stress` => sample 30 / 1500ms).
pub(crate) fn criterion_config() -> Criterion {
    let profile = BenchProfile::from_env();
    Criterion::default()
        .sample_size(profile.sample_size())
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(match profile {
            BenchProfile::Quick => 500,
            BenchProfile::Full | BenchProfile::Stress => 1_500,
            _ => 500,
        }))
}

/// Build the standard unweighted, all-labels projection from a graph snapshot —
/// the projection every algorithm bench (and the projection-build bench itself)
/// measures over.
pub(crate) fn build_projection(snapshot: &SeleneGraph) -> GraphProjection {
    GraphProjection::build(snapshot, &projection_config(), None).expect("bench projection builds")
}

/// The standard projection config: all alive nodes, all edge types, unweighted.
/// Exposed separately so the build bench can construct it inside the timed
/// routine without re-running the build.
pub(crate) fn projection_config() -> ProjectionConfig {
    ProjectionConfig {
        name: "bench".to_string(),
        node_labels: Vec::new(),
        edge_labels: Vec::new(),
        weight_property: None,
    }
}

/// Compact `10k`-style scale label for criterion benchmark ids.
pub(crate) fn scale_label(scale: usize) -> String {
    if scale >= 1_000 {
        format!("{}k", scale / 1_000)
    } else {
        scale.to_string()
    }
}
