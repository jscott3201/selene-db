//! Vector snapshot harness — closes M8 per BRIEF-64.

use std::collections::BTreeSet;
use std::path::Path;

use selene_testing::{
    ERROR_KIND_COVERAGE, ErrorInductionKind, MAGIC_COVERAGE, METRIC_COVERAGE,
    NEIGHBOR_SELECTION_COVERAGE, NeighborSelectionFlavor, OP_COVERAGE, QUANT_METHOD_COVERAGE,
    QuantMethodMirror, SURFACE_COVERAGE, VectorConfigSpec, VectorCorpus, VectorCorpusCategory,
    VectorCorpusEntry, VectorCorpusGraph, VectorCorpusInvocation, VectorErrorKindMirror,
    VectorMagicMirror, VectorMetricMirror, VectorOpMirror, VectorQuantizationSpec,
};
use selene_vector::snapshot_summary::{
    distance_metric_anchor, magic_constants, neighbor_selection_anchor, quant_method_anchor,
    vector_error_kind_for, vector_op_anchor,
};
use selene_vector::{
    DistanceMetric, HnswConfig, NeighborSelectionConfig, PAYLOAD_MAGIC, PAYLOAD_MAGIC_BULK,
    QuantMethod, VectorOp,
};

mod vector_snapshot_support;

use vector_snapshot_support::{
    canonical_error_for_kind, config_from_spec, execute_entry, provider_with_graph,
};

#[test]
fn corpus_snapshots_match() {
    for entry in VectorCorpus::m8().entries() {
        let snapshot = execute_entry(entry);
        if cfg!(feature = "simd-simsimd") {
            // Codex review fix (P2-B): upgrade the SIMD path from a bare
            // emptiness check to structural-validity assertions. Strict
            // byte-or-score parity against the scalar golden is infeasible:
            // simsimd's f32 reduction order differs from the scalar kernel
            // at IEEE-rounding-tolerance levels, which the HNSW
            // neighbor-selection heuristic amplifies into different
            // neighbor lists, different section bytes, and different
            // blake3 digests. The asserted invariants here are everything
            // that MUST hold regardless of kernel choice: non-empty
            // output, all four `====` headers present, the entry slug
            // appears in CONFIG, and no renderer-side decode errors leak
            // into the snapshot.
            assert_simd_structural_validity(entry, &snapshot.to_string());
        } else {
            insta::with_settings!({ snapshot_suffix => entry.slug }, {
                insta::assert_snapshot!(snapshot.to_string());
            });
        }
    }
}

#[test]
fn corpus_slugs_are_unique() {
    let mut seen = BTreeSet::new();
    for entry in VectorCorpus::m8().entries() {
        assert!(seen.insert(entry.slug), "duplicate slug {}", entry.slug);
    }
}

#[test]
fn corpus_categories_covered() {
    let actual = VectorCorpus::m8()
        .entries()
        .map(|entry| entry.category)
        .collect::<BTreeSet<_>>();
    for expected in VectorCorpusCategory::ALL {
        assert!(
            actual.contains(expected),
            "category {expected:?} has no corpus entry"
        );
    }
}

#[test]
fn corpus_covers_every_vector_surface() {
    let actual = covered(|entry| entry.covered_surfaces);
    for surface in SURFACE_COVERAGE {
        assert!(
            actual.contains(surface),
            "surface {} has no corpus entry",
            surface.name()
        );
    }
}

#[test]
fn corpus_covers_every_distance_metric() {
    let actual = covered(|entry| entry.covered_metrics);
    for metric in METRIC_COVERAGE {
        assert!(
            actual.contains(metric),
            "metric {} has no corpus entry",
            metric.name()
        );
    }
}

#[test]
fn corpus_covers_every_vector_op() {
    let actual = covered(|entry| entry.covered_ops);
    for op in OP_COVERAGE {
        assert!(actual.contains(op), "op {} has no corpus entry", op.name());
    }
}

#[test]
fn corpus_covers_every_vector_error_variant() {
    let actual = covered(|entry| entry.covered_errors);
    for kind in ERROR_KIND_COVERAGE {
        assert!(
            actual.contains(kind),
            "error kind {} has no corpus entry",
            kind.name()
        );
    }
}

#[test]
fn corpus_covers_every_payload_magic() {
    let actual = covered(|entry| entry.covered_magics);
    for magic in MAGIC_COVERAGE {
        assert!(
            actual.contains(magic),
            "magic {} has no corpus entry",
            magic.name()
        );
    }
}

#[test]
fn corpus_covers_every_quant_method() {
    let actual = covered(|entry| entry.covered_quant_methods);
    for method in QUANT_METHOD_COVERAGE {
        assert!(
            actual.contains(method),
            "quant method {} has no corpus entry",
            method.name()
        );
    }
}

#[test]
fn distance_metric_mirror_matches_anchor() {
    assert_anchor_names(
        VectorMetricMirror::ALL.iter().map(|metric| metric.name()),
        distance_metric_anchor().iter().map(|(name, _)| *name),
    );
}

#[test]
fn vector_error_kind_mirror_matches_kind_function() {
    let errors = VectorErrorKindMirror::ALL
        .iter()
        .copied()
        .map(canonical_error_for_kind)
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), VectorErrorKindMirror::ALL.len());
    for (mirror, error) in VectorErrorKindMirror::ALL.iter().copied().zip(&errors) {
        assert_eq!(vector_error_kind_for(error).name(), mirror.name());
    }
}

#[test]
fn vector_op_mirror_matches_anchor() {
    assert_anchor_names(
        VectorOpMirror::ALL.iter().map(|op| op.name()),
        vector_op_anchor().iter().map(|(name, _)| *name),
    );
}

#[test]
fn payload_magic_mirror_matches_constants() {
    assert_anchor_names(
        VectorMagicMirror::ALL.iter().map(|magic| magic.name()),
        magic_constants().iter().map(|(name, _)| *name),
    );
    let expected = [
        ("VECU", PAYLOAD_MAGIC),
        ("VECB", PAYLOAD_MAGIC_BULK),
        ("VGRP", *b"VGRP"),
        ("VVEC", *b"VVEC"),
        ("VQNT", *b"VQNT"),
    ];
    assert_eq!(magic_constants(), expected);
}

#[test]
fn quant_method_mirror_matches_anchor() {
    assert_anchor_names(
        QuantMethodMirror::ALL.iter().map(|method| method.name()),
        quant_method_anchor().iter().map(|(name, _)| *name),
    );
}

#[test]
fn neighbor_select_flavor_mirror_drift() {
    assert_anchor_names(
        NeighborSelectionFlavor::ALL
            .iter()
            .map(|flavor| flavor.name()),
        neighbor_selection_anchor().iter().map(|(name, _)| *name),
    );
    assert_eq!(
        NEIGHBOR_SELECTION_COVERAGE.len(),
        neighbor_selection_anchor().len()
    );
}

#[test]
fn corpus_diversity_entries_cover_all_flavors() {
    let actual = VectorCorpus::m8()
        .entries()
        .map(|entry| entry.config.neighbor_selection_flavor)
        .collect::<BTreeSet<_>>();
    for flavor in NeighborSelectionFlavor::ALL {
        assert!(
            actual.contains(flavor),
            "neighbor selection flavor {} has no corpus entry",
            flavor.name()
        );
    }
}

#[test]
fn hnsw_config_neighbor_selection_field_is_public() {
    let config = HnswConfig {
        dim: 4,
        m: 16,
        ef_construction: 200,
        ef_search: 50,
        metric: DistanceMetric::Cosine,
        quantization: Default::default(),
        neighbor_selection: NeighborSelectionConfig::default(),
    };
    assert_eq!(
        config.neighbor_selection,
        NeighborSelectionConfig {
            extend_candidates: false,
            keep_pruned_connections: true
        }
    );
}

#[test]
fn corpus_fixture_graphs_build_successfully() {
    for graph in VectorCorpusGraph::ALL {
        let config = match graph {
            VectorCorpusGraph::DeterministicL2_100
            | VectorCorpusGraph::DiverseClusterL2_64
            | VectorCorpusGraph::DenseClusterL2_8 => config_from_spec(VectorConfigSpec::new(
                8,
                VectorMetricMirror::Cosine,
                VectorQuantizationSpec::DISABLED,
            )),
            _ => config_from_spec(VectorConfigSpec::new(
                4,
                VectorMetricMirror::Cosine,
                VectorQuantizationSpec::DISABLED,
            )),
        };
        let _ = provider_with_graph(*graph, config);
    }
}

#[test]
fn error_fixtures_split_api_vs_synthetic() {
    let synthetic_only = [
        VectorErrorKindMirror::DimensionMismatch,
        VectorErrorKindMirror::SectionDecodeFailed,
        VectorErrorKindMirror::SectionEncodeFailed,
        VectorErrorKindMirror::EncodeFailed,
        VectorErrorKindMirror::InternalIndexExhausted,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();

    for entry in VectorCorpus::m8().entries() {
        let Some(kind) = entry.invocation.induction_kind() else {
            continue;
        };
        let expected = match &entry.invocation {
            VectorCorpusInvocation::DeliberateApiError { kind, .. }
                if synthetic_only.contains(kind) =>
            {
                panic!("synthetic-only kind {} appears as Api", kind.name());
            }
            VectorCorpusInvocation::DeliberateSyntheticError { kind, .. }
                if !synthetic_only.contains(kind) =>
            {
                panic!("api-reachable kind {} appears as Synthetic", kind.name());
            }
            VectorCorpusInvocation::DeliberateApiError { .. } => ErrorInductionKind::Api,
            VectorCorpusInvocation::DeliberateSyntheticError { .. } => {
                ErrorInductionKind::Synthetic
            }
            _ => unreachable!("induction_kind filtered non-error invocation"),
        };
        assert_eq!(kind, expected, "{} induction kind drift", entry.slug);
    }
}

#[test]
fn vector_corpus_has_no_selene_vector_imports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../selene-testing/src/vector_corpus");
    for path in rust_files(&root) {
        let contents = std::fs::read_to_string(&path).expect("read vector corpus source");
        for (line_no, line) in contents.lines().enumerate() {
            assert!(
                !line.trim_start().starts_with("use selene_vector"),
                "{}:{} imports selene_vector",
                path.display(),
                line_no + 1
            );
        }
    }
}

#[test]
fn target_vec_param_removed() {
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hnsw/build.rs"))
            .expect("read build.rs");

    assert!(!source.contains("_target_vec"));
    assert!(!source.contains("target_vec:"));
}

#[test]
fn prune_never_extends() {
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hnsw/build.rs"))
            .expect("read build.rs");
    let prune_body = source
        .split("fn prune_neighbors")
        .nth(1)
        .expect("prune_neighbors exists");

    assert!(prune_body.contains("extend_candidates: false"));
}

#[test]
fn bench_invocation_hygiene_is_declared() {
    let cargo = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("read selene-vector Cargo.toml");

    assert!(cargo.contains("autobenches = false"));
    assert!(cargo.contains("name = \"recall\""));
    assert!(cargo.contains("harness = false"));
}

#[test]
fn scripts_run_benches_includes_recall_entry() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-benches.sh"),
    )
    .expect("read run-benches.sh");

    assert!(script.contains("selene-vector:recall:criterion"));
}

#[test]
fn proptest_workspace_dev_dep_present() {
    let cargo = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("read selene-vector Cargo.toml");

    assert!(cargo.contains("proptest.workspace = true"));
}

#[test]
fn recall_bench_uses_public_provider_search() {
    let bench =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/recall.rs"))
            .expect("read recall bench");

    assert!(bench.contains(".search(query, k, Some(ef_search), None)"));
    assert!(!bench.contains("hnsw::search::search"));
}

#[test]
fn use_import_gate_mentions_public_vector_surface() {
    let _ = (
        DistanceMetric::Cosine,
        VectorOp::Insert,
        QuantMethod::Sq8,
        PAYLOAD_MAGIC,
        PAYLOAD_MAGIC_BULK,
    );
}

fn covered<T>(field: impl Fn(&VectorCorpusEntry) -> &'static [T]) -> BTreeSet<T>
where
    T: Copy + Ord + 'static,
{
    let mut actual = BTreeSet::new();
    for entry in VectorCorpus::m8().entries() {
        actual.extend(field(entry).iter().copied());
    }
    actual
}

fn assert_anchor_names<'a>(
    expected: impl Iterator<Item = &'a str>,
    observed: impl Iterator<Item = &'a str>,
) {
    let expected = expected.collect::<Vec<_>>();
    let observed = observed.collect::<Vec<_>>();
    assert_eq!(observed, expected);
}

/// Codex review fix (P2-B): the SIMD path asserts structural validity of
/// the rendered snapshot — invariants that hold regardless of kernel
/// choice. Strict byte parity against the scalar golden is infeasible
/// because simsimd's reduction order differs from the scalar kernel at
/// IEEE-rounding tolerance, and HNSW's score-driven neighbor selection
/// amplifies that into different neighbor lists, section bytes, and
/// blake3 digests. The smoke checked here:
///   1. The rendered text is non-empty.
///   2. All four `====` block headers are present in order.
///   3. The entry slug appears in the CONFIG block.
///   4. No `decode_error=` token leaks into the snapshot (renderer-side
///      decode errors would surface here without one).
fn assert_simd_structural_validity(entry: &VectorCorpusEntry, simd: &str) {
    assert!(!simd.is_empty(), "SIMD output empty for {}", entry.slug);

    let block_headers = [
        "==== CONFIG ====",
        "==== GRAPH ====",
        "==== SECTIONS ====",
        "==== RESULT ====",
    ];
    let mut cursor = 0;
    for header in block_headers {
        let position = simd[cursor..]
            .find(header)
            .unwrap_or_else(|| panic!("SIMD output missing {header} for {}", entry.slug));
        cursor += position + header.len();
    }

    let expected_slug_line = format!("slug=\"{}\"", entry.slug);
    assert!(
        simd.contains(&expected_slug_line),
        "SIMD output missing {expected_slug_line} for {}",
        entry.slug
    );

    assert!(
        !simd.contains("decode_error="),
        "SIMD output leaked a renderer decode_error for {}",
        entry.slug
    );
}

fn rust_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(root).expect("read corpus dir");
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}
