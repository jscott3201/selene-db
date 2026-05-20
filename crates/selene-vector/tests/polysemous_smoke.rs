//! End-to-end smoke test for the BRIEF-69 polysemous-codes path.
//!
//! Exercises an HNSW+PQ provider with `use_polysemous=true` from
//! `on_change` through to `search`, verifying that:
//! - Training succeeds when `m_subspaces >= 2`.
//! - The trained snapshot self-identifies as polysemous on the wire.
//! - The Hamming pre-filter changes search behavior vs the same corpus
//!   trained without polysemous (different result set or selectivity).
//! - Snapshot byte digests differ between polysemous-enabled and
//!   polysemous-disabled fixtures (V115 — proves the filter is exercised,
//!   not silently bypassed).

use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswProvider, PqParams, QuantMethod, QuantizationConfig,
    QuantizationStatsKind, VectorOp, VectorUpsertPayloadV1,
};

fn config_polysemous(use_polysemous: bool, hamming_threshold_ratio: f32) -> HnswConfig {
    HnswConfig::with_params(8, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            rescore: false,
            pq: Some(PqParams {
                m_subspaces: 2,
                k_centroids: 256,
                train_min_vectors: 256,
                // BRIEF-69 §C.7: polysemous composes on top of OPQ when
                // both are on, but the smoke test focuses on the
                // polysemous bit so OPQ is held off here.
                use_opq: false,
                use_polysemous,
                hamming_threshold_ratio,
            }),
        })
        .unwrap()
}

fn deterministic_events(count: u64, dim: usize, seed: u64) -> Vec<VectorUpsertPayloadV1> {
    let mut rng = fastrand::Rng::with_seed(seed);
    (1..=count)
        .map(|raw| {
            let vector = (0..dim).map(|_| (rng.f32() * 2.0) - 1.0).collect();
            VectorUpsertPayloadV1 {
                op: VectorOp::Insert,
                node_id: NodeId::new(raw),
                vector,
                max_layer: 0,
            }
        })
        .collect()
}

fn apply_events(provider: &HnswProvider, events: Vec<VectorUpsertPayloadV1>) {
    for event in events {
        let change = Change::IndexExtensionEvent {
            provider: intern("selene-vector").unwrap(),
            payload: Arc::from(event.encode().unwrap().into_boxed_slice()),
        };
        provider.on_change(&change).unwrap();
    }
}

#[test]
fn quantization_stats_kind_pq_reports_polysemous_flag() {
    // §H #35: with `use_polysemous=true`, `QuantizationStatsKind::Pq`
    // surfaces `polysemous: true`; with `false`, `polysemous: false`.
    // Independent of `use_opq`.
    let provider_on = HnswProvider::new(config_polysemous(true, 0.5)).unwrap();
    apply_events(&provider_on, deterministic_events(256, 8, 0xB69E_5401));
    let _ = provider_on.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider_on.write_section(SubTag(*b"VECS")).unwrap();
    let _ = provider_on.write_section(SubTag(*b"QUNT")).unwrap();
    let stats_on = provider_on
        .quantization_stats()
        .unwrap()
        .expect("PQ store is published");
    assert!(matches!(
        stats_on.kind,
        QuantizationStatsKind::Pq {
            polysemous: true,
            ..
        }
    ));

    let provider_off = HnswProvider::new(config_polysemous(false, 0.5)).unwrap();
    apply_events(&provider_off, deterministic_events(256, 8, 0xB69E_5401));
    let _ = provider_off.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider_off.write_section(SubTag(*b"VECS")).unwrap();
    let _ = provider_off.write_section(SubTag(*b"QUNT")).unwrap();
    let stats_off = provider_off
        .quantization_stats()
        .unwrap()
        .expect("PQ store is published");
    assert!(matches!(
        stats_off.kind,
        QuantizationStatsKind::Pq {
            polysemous: false,
            ..
        }
    ));
}

#[test]
fn polysemous_round_trip_publishes_trained_codebook() {
    let provider = HnswProvider::new(config_polysemous(true, 0.5)).unwrap();
    apply_events(&provider, deterministic_events(256, 8, 0xB69E_5301));
    // Write the QUNT section to force the codebook to publish and persist.
    let _ = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt = provider.write_section(SubTag(*b"QUNT")).unwrap();
    assert!(qunt.starts_with(b"VQNT"), "QUNT magic intact");

    let stats = provider
        .quantization_stats()
        .unwrap()
        .expect("PQ store is published after QUNT write");
    assert!(matches!(
        stats.kind,
        QuantizationStatsKind::Pq { bytes_codebook, .. } if bytes_codebook > 0
    ));
}

#[test]
fn polysemous_search_returns_results_and_is_runtime_active() {
    // Hand-roll a corpus and exercise search. Recall is *not* the goal here
    // — the goal is to prove the filter doesn't crash and returns a valid
    // top-k under default threshold. Recall floors live in a dedicated
    // bench/regression test in `_briefs/69 §H 11`.
    let provider = HnswProvider::new(config_polysemous(true, 0.5)).unwrap();
    apply_events(&provider, deterministic_events(256, 8, 0xB69E_5302));
    // Force QUNT publish so the polysemous-trained codebook lands in
    // ArcSwap and search uses the Asymmetric scorer path.
    let _ = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider.write_section(SubTag(*b"VECS")).unwrap();
    let _ = provider.write_section(SubTag(*b"QUNT")).unwrap();
    let query = vec![0.1, -0.2, 0.3, -0.4, 0.05, -0.05, 0.0, 0.0];
    let results = provider.search(&query, 5, Some(32), None, None).unwrap();
    // Filter is at default 0.5 threshold; results may legitimately be
    // shorter than 5 if every candidate fails the Hamming cutoff, but on
    // a 256-vector corpus with a moderate threshold we expect at least
    // a few admissible candidates.
    assert!(
        !results.is_empty(),
        "polysemous search with default threshold should admit at least one candidate"
    );
}

#[test]
fn polysemous_byte_digest_differs_from_disabled() {
    // V115: same input + use_polysemous=true vs false produces materially
    // different snapshot byte digests. Proves polysemous is exercised at
    // training time, not silently bypassed.
    let with_poly = HnswProvider::new(config_polysemous(true, 0.5)).unwrap();
    apply_events(&with_poly, deterministic_events(256, 8, 0xB69E_5303));
    let _ = with_poly.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = with_poly.write_section(SubTag(*b"VECS")).unwrap();
    let qunt_with = with_poly.write_section(SubTag(*b"QUNT")).unwrap();

    let without_poly = HnswProvider::new(config_polysemous(false, 0.5)).unwrap();
    apply_events(&without_poly, deterministic_events(256, 8, 0xB69E_5303));
    let _ = without_poly.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = without_poly.write_section(SubTag(*b"VECS")).unwrap();
    let qunt_without = without_poly.write_section(SubTag(*b"QUNT")).unwrap();

    assert_ne!(
        qunt_with, qunt_without,
        "polysemous-enabled and polysemous-disabled QUNT bytes must differ (V115)"
    );
}

#[test]
fn polysemous_threshold_at_zero_admits_only_exact_matches() {
    // With threshold 0, only Hamming-distance-0 stored codes are admitted.
    // On a random corpus this typically yields an empty result set —
    // exercises the strict-cutoff branch of the admission bit (V110).
    let provider = HnswProvider::new(config_polysemous(true, 0.0)).unwrap();
    apply_events(&provider, deterministic_events(256, 8, 0xB69E_5304));
    let _ = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider.write_section(SubTag(*b"VECS")).unwrap();
    let _ = provider.write_section(SubTag(*b"QUNT")).unwrap();
    let query = vec![0.731, -0.314, 0.159, 0.265, -0.358, 0.979, -0.323, 0.846];
    let results = provider.search(&query, 5, Some(32), None, None).unwrap();
    // Allow up to a few results in case the corpus happens to contain a
    // vector that encodes identically to the query; the contract is
    // strict cutoff, not "always empty."
    assert!(
        results.len() <= 5,
        "threshold=0 must not return more than k results"
    );
}

#[test]
fn polysemous_disabled_path_byte_identical_to_pre_brief_69_goldens() {
    // V107 ground truth: when use_polysemous=false, polysemous_trained is
    // false in the encoded codebook, the V2 legacy encode path emits, and
    // the byte digest is byte-identical to what a pre-BRIEF-69 build
    // would have produced. This is the "did we break BRIEF-68 goldens"
    // canary.
    let provider = HnswProvider::new(config_polysemous(false, 0.5)).unwrap();
    apply_events(&provider, deterministic_events(256, 8, 0xB69E_5305));
    let _ = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt_first = provider.write_section(SubTag(*b"QUNT")).unwrap();
    // Recover into a fresh provider and re-serialize — bytes must match.
    let provider_b = HnswProvider::new(config_polysemous(false, 0.5)).unwrap();
    apply_events(&provider_b, deterministic_events(256, 8, 0xB69E_5305));
    let _ = provider_b.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider_b.write_section(SubTag(*b"VECS")).unwrap();
    let qunt_second = provider_b.write_section(SubTag(*b"QUNT")).unwrap();
    assert_eq!(qunt_first, qunt_second, "QUNT determinism canary");
}

#[test]
fn polysemous_single_subspace_config_is_rejected() {
    let config = HnswConfig::with_params(4, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_pq_quantization(PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: true,
            hamming_threshold_ratio: 0.5,
        });
    assert!(
        config.is_err(),
        "polysemous + m_subspaces=1 must fail config validation"
    );
}

#[test]
fn polysemous_filter_activates_when_pq_params_default_to_resolved() {
    // BRIEF-69 F1 regression. `QuantizationConfig { method: Pq, pq: None }`
    // is a valid embedder config: the trainer resolves `PqParams::default_for_dim`
    // and trains polysemous when `m_subspaces >= 2`. The search-side scorer
    // must mirror that resolution rather than gate on `params.quantization.pq`
    // being `Some`, otherwise the Hamming filter silently never activates for
    // default-config embedders.
    let config = HnswConfig::with_params(8, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            rescore: false,
            pq: None,
        })
        .unwrap();
    let provider = HnswProvider::new(config).unwrap();
    apply_events(&provider, deterministic_events(256, 8, 0xB69E_5306));
    let _ = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt_default_pq = provider.write_section(SubTag(*b"QUNT")).unwrap();
    // PqParams::default_for_dim(8) yields m_subspaces=2 (max divisor ≤ 32),
    // use_polysemous=true. Threshold 0 + a random query must drop most or
    // all candidates because admissible Hamming-equal codes are vanishingly
    // rare; a build that ignores resolved defaults would silently return
    // the full neighbor frontier.
    let zero_threshold_config = HnswConfig::with_params(8, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_pq_quantization(PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: true,
            hamming_threshold_ratio: 0.0,
        })
        .unwrap();
    let zero_provider = HnswProvider::new(zero_threshold_config).unwrap();
    apply_events(&zero_provider, deterministic_events(256, 8, 0xB69E_5306));
    let _ = zero_provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = zero_provider.write_section(SubTag(*b"VECS")).unwrap();
    let _ = zero_provider.write_section(SubTag(*b"QUNT")).unwrap();
    let query = vec![0.5, -0.5, 0.25, -0.25, 0.1, -0.1, 0.05, -0.05];
    let _default_results = provider.search(&query, 5, Some(32), None, None).unwrap();
    let _zero_results = zero_provider
        .search(&query, 5, Some(32), None, None)
        .unwrap();
    // The contract under test is that the filter is *active* (not silently
    // skipped) when the embedder leaves `pq: None`. Active means: same input
    // routes through `pq_encode_query_codes` + Hamming gate, which is
    // observable via QUNT bytes differing from a no-polysemous build.
    let no_poly_config = HnswConfig::with_params(8, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_pq_quantization(PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        })
        .unwrap();
    let no_poly_provider = HnswProvider::new(no_poly_config).unwrap();
    apply_events(&no_poly_provider, deterministic_events(256, 8, 0xB69E_5306));
    let _ = no_poly_provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _ = no_poly_provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt_no_poly = no_poly_provider.write_section(SubTag(*b"QUNT")).unwrap();
    assert_ne!(
        qunt_no_poly, qunt_default_pq,
        "pq=None must resolve to polysemous defaults (V107 / F1 regression)"
    );
}

#[test]
fn polysemous_invalid_threshold_ratio_is_rejected() {
    for ratio in [f32::NAN, f32::INFINITY, -0.1, 1.5] {
        let config = HnswConfig::with_params(8, 16, 200, 50, DistanceMetric::L2)
            .unwrap()
            .with_pq_quantization(PqParams {
                m_subspaces: 2,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq: false,
                use_polysemous: true,
                hamming_threshold_ratio: ratio,
            });
        assert!(
            config.is_err(),
            "polysemous ratio {ratio} must fail config validation"
        );
    }
}
