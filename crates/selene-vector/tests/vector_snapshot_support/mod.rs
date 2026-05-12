use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_testing::{
    ApiInductionPayload, ErrorInductionKind, NeighborSelectionFlavor, QuantMethodMirror,
    SyntheticErrorFields, VectorConfigSpec, VectorCorpusEntry, VectorCorpusEvent,
    VectorCorpusGraph, VectorCorpusInvocation, VectorErrorKindMirror, VectorMetricMirror,
};
use selene_vector::hnsw::{HnswGraph, HnswParams, insert_node};
use selene_vector::snapshot_summary::{
    QuantizationParitySummary, QuantizationStatsSummary, SearchRowsSummary, VectorConfigSummary,
    VectorGraphSummary, VectorInvocationResult, VectorSectionsSummary, VectorSnapshot,
    VectorSnapshotInput, render_vector_error, vector_error_kind_for, vector_summary,
};
use selene_vector::{
    BulkInsertRow, DistanceMetric, HnswConfig, HnswProvider, NeighborSelectionConfig, PqParams,
    QuantMethod, QuantizationConfig, VectorBulkInsertPayloadV1, VectorError, VectorOp,
    VectorUpsertPayloadV1,
};

pub(crate) fn execute_entry(entry: &VectorCorpusEntry) -> VectorSnapshot {
    let config = config_from_spec(entry.config);
    let source = provider_with_graph(entry.graph, config.clone());
    let source_graph = source.snapshot();
    let graph = VectorGraphSummary::from_graph(&source_graph);
    let sections = sections_for(&source);
    assert_byte_identity(entry, &sections);

    let result = match &entry.invocation {
        VectorCorpusInvocation::SnapshotRoundtrip => {
            let recovered = recover_provider(&config, &sections);
            let recovered_graph = recovered.snapshot();
            VectorInvocationResult::Roundtrip {
                recovered_matches_source: graph == VectorGraphSummary::from_graph(&recovered_graph),
            }
        }
        VectorCorpusInvocation::Search {
            query,
            k,
            ef_search,
            filter,
        } if entry.config.quantization.enabled => {
            let baseline_config = config
                .clone()
                .with_quantization(QuantizationConfig::default())
                .expect("baseline config is valid");
            let baseline_source = provider_with_graph(entry.graph, baseline_config);
            let baseline = baseline_source
                .search(query, *k, *ef_search, bitmap(filter).as_ref())
                .expect("baseline search succeeds");
            let asymmetric_config = config
                .clone()
                .with_quantization(QuantizationConfig {
                    enabled: true,
                    rescore: false,
                    method: config.quantization.method,
                    pq: config.quantization.pq,
                })
                .expect("asymmetric config is valid");
            let rescored_config = config
                .clone()
                .with_quantization(QuantizationConfig {
                    enabled: true,
                    rescore: true,
                    method: config.quantization.method,
                    pq: config.quantization.pq,
                })
                .expect("rescored config is valid");
            let asymmetric = recover_provider(&asymmetric_config, &sections);
            let rescored = recover_provider(&rescored_config, &sections);
            let filter = bitmap(filter);
            let sq8 = asymmetric
                .search(query, *k, *ef_search, filter.as_ref())
                .expect("SQ8 search succeeds");
            let rescore = rescored
                .search(query, *k, *ef_search, filter.as_ref())
                .expect("SQ8 rescore search succeeds");
            VectorInvocationResult::SearchParity {
                parity: QuantizationParitySummary {
                    f32_baseline: SearchRowsSummary::from_rows(&baseline),
                    sq8_asymmetric: SearchRowsSummary::from_rows(&sq8),
                    sq8_rescored: SearchRowsSummary::from_rows(&rescore),
                },
            }
        }
        VectorCorpusInvocation::Search {
            query,
            k,
            ef_search,
            filter,
        } => {
            let filter = bitmap(filter);
            let rows = source
                .search(query, *k, *ef_search, filter.as_ref())
                .expect("search succeeds");
            VectorInvocationResult::Search {
                rows: SearchRowsSummary::from_rows(&rows),
            }
        }
        VectorCorpusInvocation::RecoveryReplay {
            post_snapshot_events,
        } => {
            let recovered = recover_provider(&config, &sections);
            apply_events(&recovered, post_snapshot_events);
            let from_scratch = provider_with_graph(entry.graph, config.clone());
            apply_events(&from_scratch, post_snapshot_events);
            let recovered_sections = sections_for(&recovered);
            let scratch_sections = sections_for(&from_scratch);
            VectorInvocationResult::Replay {
                post_replay_summary: VectorGraphSummary::from_graph(&recovered.snapshot()),
                byte_identical: recovered_sections.grph == scratch_sections.grph
                    && recovered_sections.vecs == scratch_sections.vecs
                    && recovered_sections.qunt == scratch_sections.qunt,
            }
        }
        VectorCorpusInvocation::StatsOnly => {
            let recovered = recover_provider(&config, &sections);
            let stats = recovered
                .quantization_stats()
                .expect("QUNT stats API succeeds")
                .expect("QUNT stats are present");
            VectorInvocationResult::Stats {
                stats: QuantizationStatsSummary::from_stats(stats),
            }
        }
        VectorCorpusInvocation::DeliberateApiError { kind, payload } => {
            let error = induce_api_error(payload, &source);
            assert_eq!(vector_error_kind_for(&error).name(), kind.name());
            VectorInvocationResult::Error {
                kind: vector_error_kind_for(&error),
                induction_kind: ErrorInductionKind::Api.name(),
                rendered: render_vector_error(&error),
            }
        }
        VectorCorpusInvocation::DeliberateSyntheticError { kind, fields } => {
            let error = synthetic_error(fields);
            assert_eq!(vector_error_kind_for(&error).name(), kind.name());
            VectorInvocationResult::Error {
                kind: vector_error_kind_for(&error),
                induction_kind: ErrorInductionKind::Synthetic.name(),
                rendered: render_vector_error(&error),
            }
        }
        _ => panic!("unknown vector corpus invocation in {}", entry.slug),
    };

    vector_summary(&VectorSnapshotInput {
        slug: entry.slug.to_string(),
        description: entry.description.to_string(),
        config: VectorConfigSummary::from_config(&config),
        graph,
        sections: match result {
            VectorInvocationResult::Error { .. } => None,
            _ => Some(sections),
        },
        invocation_result: result,
    })
}

pub(crate) fn provider_with_graph(graph: VectorCorpusGraph, config: HnswConfig) -> HnswProvider {
    let provider = HnswProvider::new(config.clone()).expect("provider config is valid");
    apply_events(&provider, &events_for_graph(graph, config.dim));
    provider
}

fn events_for_graph(graph: VectorCorpusGraph, dim: usize) -> Vec<VectorCorpusEvent> {
    match graph {
        VectorCorpusGraph::Empty => Vec::new(),
        VectorCorpusGraph::SingleOriginCosine => vec![event_insert(1, vec![1.0, 0.0, 0.0, 0.0], 0)],
        VectorCorpusGraph::OrthogonalBasisCosine4 => vec![
            event_insert(1, vec![1.0, 0.0, 0.0, 0.0], 0),
            event_insert(2, vec![0.0, 1.0, 0.0, 0.0], 0),
            event_insert(3, vec![0.0, 0.0, 1.0, 0.0], 0),
            event_insert(4, vec![0.0, 0.0, 0.0, 1.0], 0),
        ],
        VectorCorpusGraph::DeterministicL2_100 => {
            let rows = deterministic_rows(1, 100, dim, 42, true);
            vec![VectorCorpusEvent::Bulk { rows }]
        }
        VectorCorpusGraph::MixedLayerCosine30 => deterministic_rows(1, 30, dim, 64, true)
            .into_iter()
            .map(|(raw, vector, layer)| event_insert(raw, vector, layer))
            .collect(),
        VectorCorpusGraph::QuantizedPrefixCosine => deterministic_rows(1, 50, dim, 99, false)
            .into_iter()
            .map(|(raw, vector, layer)| event_insert(raw, vector, layer))
            .collect(),
        VectorCorpusGraph::PqTrainingL2_256 => {
            vec![VectorCorpusEvent::Bulk {
                rows: deterministic_rows(1, 256, dim, 166, true),
            }]
        }
        VectorCorpusGraph::DiverseClusterL2_64 => {
            vec![VectorCorpusEvent::Bulk {
                rows: diverse_cluster_rows(dim),
            }]
        }
        VectorCorpusGraph::DenseClusterL2_8 => {
            vec![VectorCorpusEvent::Bulk {
                rows: dense_cluster_rows(dim),
            }]
        }
        _ => panic!("unknown vector corpus graph"),
    }
}

fn deterministic_rows(
    start: u64,
    count: usize,
    dim: usize,
    seed: u64,
    mixed_layers: bool,
) -> Vec<(u64, Vec<f32>, u8)> {
    let mut rng = fastrand::Rng::with_seed(seed);
    (0..count)
        .map(|offset| {
            let raw = start + offset as u64;
            let vector = (0..dim)
                .map(|coord| {
                    let jitter = (rng.f32() * 2.0) - 1.0;
                    jitter + ((raw as f32 + coord as f32) * 0.013)
                })
                .collect();
            let layer = if mixed_layers && raw % 17 == 0 {
                2
            } else if mixed_layers && raw % 5 == 0 {
                1
            } else {
                0
            };
            (raw, vector, layer)
        })
        .collect()
}

fn diverse_cluster_rows(dim: usize) -> Vec<(u64, Vec<f32>, u8)> {
    let centers = [[0.0, 0.0], [8.0, 0.0], [0.0, 8.0], [8.0, 8.0]];
    let mut rows = Vec::new();
    for (cluster, center) in centers.iter().enumerate() {
        for member in 0..16 {
            let raw = (cluster * 16 + member + 1) as u64;
            let mut vector = vec![0.0; dim];
            vector[0] = center[0] + (member as f32 * 0.021);
            vector[1] = center[1] - (member as f32 * 0.017);
            for (coord, value) in vector.iter_mut().enumerate().skip(2) {
                *value = ((cluster + 1) as f32 * 0.13) + (coord as f32 * 0.011);
            }
            let layer = if raw % 13 == 0 {
                2
            } else if raw % 5 == 0 {
                1
            } else {
                0
            };
            rows.push((raw, vector, layer));
        }
    }
    rows
}

fn dense_cluster_rows(dim: usize) -> Vec<(u64, Vec<f32>, u8)> {
    (0..8)
        .map(|offset| {
            let raw = (offset + 1) as u64;
            let mut vector = vec![0.0; dim];
            for (coord, value) in vector.iter_mut().enumerate() {
                *value = (offset as f32 * 0.01) + (coord as f32 * 0.001);
            }
            (raw, vector, 0)
        })
        .collect()
}

pub(crate) fn config_from_spec(spec: VectorConfigSpec) -> HnswConfig {
    HnswConfig::with_params(
        spec.dim,
        spec.m,
        spec.ef_construction,
        spec.ef_search,
        metric_from_mirror(spec.metric),
    )
    .expect("base config is valid")
    .with_neighbor_selection(neighbor_selection_from_mirror(
        spec.neighbor_selection_flavor,
    ))
    .expect("neighbor selection config is valid")
    .with_quantization(QuantizationConfig {
        enabled: spec.quantization.enabled,
        rescore: spec.quantization.rescore,
        method: quant_method_from_spec(spec.quantization.method),
        pq: pq_params_from_spec(spec.quantization.pq),
    })
    .expect("quantization config is valid")
}

fn metric_from_mirror(metric: VectorMetricMirror) -> DistanceMetric {
    match metric {
        VectorMetricMirror::Cosine => DistanceMetric::Cosine,
        VectorMetricMirror::L2 => DistanceMetric::L2,
        VectorMetricMirror::Dot => DistanceMetric::Dot,
        _ => panic!("unknown vector metric mirror"),
    }
}

fn neighbor_selection_from_mirror(flavor: NeighborSelectionFlavor) -> NeighborSelectionConfig {
    match flavor {
        NeighborSelectionFlavor::Default => NeighborSelectionConfig::default(),
        NeighborSelectionFlavor::ExtendCandidates => NeighborSelectionConfig {
            extend_candidates: true,
            keep_pruned_connections: true,
        },
        NeighborSelectionFlavor::NoFillBack => NeighborSelectionConfig {
            extend_candidates: false,
            keep_pruned_connections: false,
        },
        NeighborSelectionFlavor::ExtendNoFillBack => NeighborSelectionConfig {
            extend_candidates: true,
            keep_pruned_connections: false,
        },
        _ => panic!("unknown neighbor selection flavor"),
    }
}

fn quant_method_from_spec(method: QuantMethodMirror) -> QuantMethod {
    match method {
        QuantMethodMirror::Sq8 => QuantMethod::Sq8,
        QuantMethodMirror::Pq => QuantMethod::Pq,
        _ => panic!("unknown quant method mirror"),
    }
}

fn pq_params_from_spec(spec: Option<selene_testing::VectorPqSpec>) -> Option<PqParams> {
    spec.map(|pq| PqParams {
        m_subspaces: pq.m_subspaces,
        k_centroids: pq.k_centroids,
        train_min_vectors: pq.train_min_vectors,
    })
}

fn apply_events(provider: &HnswProvider, events: &[VectorCorpusEvent]) {
    for event in events {
        provider
            .on_change(&change_for_event(event))
            .expect("vector event applies");
    }
}

fn change_for_event(event: &VectorCorpusEvent) -> Change {
    let payload = match event {
        VectorCorpusEvent::Insert {
            node_id_raw,
            vector,
            max_layer,
        } => VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new(*node_id_raw),
            vector: vector.clone(),
            max_layer: *max_layer,
        }
        .encode()
        .expect("VECU encodes"),
        VectorCorpusEvent::Bulk { rows } => VectorBulkInsertPayloadV1 {
            rows: rows
                .iter()
                .map(|(node_id_raw, vector, max_layer)| BulkInsertRow {
                    node_id: NodeId::new(*node_id_raw),
                    vector: vector.clone(),
                    max_layer: *max_layer,
                })
                .collect(),
        }
        .encode()
        .expect("VECB encodes"),
        _ => panic!("unknown vector corpus event"),
    };
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(payload.into_boxed_slice()),
    }
}

fn event_insert(node_id_raw: u64, vector: Vec<f32>, max_layer: u8) -> VectorCorpusEvent {
    VectorCorpusEvent::Insert {
        node_id_raw,
        vector,
        max_layer,
    }
}

fn sections_for(provider: &HnswProvider) -> VectorSectionsSummary {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt = provider.write_section(SubTag(*b"QUNT")).unwrap();
    VectorSectionsSummary::new(grph, vecs, Some(qunt))
}

fn recover_provider(config: &HnswConfig, sections: &VectorSectionsSummary) -> HnswProvider {
    let provider = HnswProvider::new(config.clone()).expect("provider config is valid");
    provider
        .read_section(SubTag(*b"GRPH"), &sections.grph)
        .unwrap();
    provider
        .read_section(SubTag(*b"VECS"), &sections.vecs)
        .unwrap();
    if let Some(qunt) = &sections.qunt {
        provider.read_section(SubTag(*b"QUNT"), qunt).unwrap();
    }
    provider
}

fn assert_byte_identity(entry: &VectorCorpusEntry, sections: &VectorSectionsSummary) {
    let again = provider_with_graph(entry.graph, config_from_spec(entry.config));
    let again_sections = sections_for(&again);
    assert_eq!(
        sections.grph, again_sections.grph,
        "{} GRPH drift",
        entry.slug
    );
    assert_eq!(
        sections.vecs, again_sections.vecs,
        "{} VECS drift",
        entry.slug
    );
    assert_eq!(
        sections.qunt, again_sections.qunt,
        "{} QUNT drift",
        entry.slug
    );
}

fn bitmap(filter: &Option<Vec<u64>>) -> Option<RoaringBitmap> {
    filter.as_ref().map(|raws| {
        let mut bitmap = RoaringBitmap::new();
        for raw in raws {
            if let Ok(raw) = u32::try_from(*raw) {
                bitmap.insert(raw);
            }
        }
        bitmap
    })
}

fn induce_api_error(payload: &ApiInductionPayload, provider: &HnswProvider) -> VectorError {
    match payload {
        ApiInductionPayload::InvalidConfigZeroDim => {
            HnswConfig::new(0).expect_err("zero dim rejected")
        }
        ApiInductionPayload::InvalidNodeIdTombstone => {
            selene_vector::HnswNode::new(NodeId::TOMBSTONE, Arc::from([0.0, 0.0, 0.0, 0.0]), 0)
                .expect_err("tombstone rejected")
        }
        ApiInductionPayload::DimensionsLockedSearch => provider
            .search(&[1.0], 1, Some(4), None)
            .expect_err("wrong dim rejected"),
        ApiInductionPayload::InvalidPayloadEmptyBulk => VectorBulkInsertPayloadV1 { rows: vec![] }
            .encode()
            .expect_err("empty bulk rejected"),
        ApiInductionPayload::OperationUpdate => operation_not_supported_error(
            provider,
            VectorUpsertPayloadV1 {
                op: VectorOp::Update,
                node_id: NodeId::new(1),
                vector: vec![1.0, 0.0, 0.0, 0.0],
                max_layer: 0,
            },
        ),
        ApiInductionPayload::OperationDelete => operation_not_supported_error(
            provider,
            VectorUpsertPayloadV1 {
                op: VectorOp::Delete,
                node_id: NodeId::new(1),
                vector: Vec::new(),
                max_layer: 0,
            },
        ),
        ApiInductionPayload::DuplicateNodeId => {
            let config = HnswConfig::new(4).unwrap();
            let params = HnswParams::from_config(&config);
            let mut graph = HnswGraph::empty(4);
            insert_node(
                &mut graph,
                NodeId::new(1),
                Arc::from([1.0, 0.0, 0.0, 0.0]),
                0,
                &params,
            )
            .unwrap();
            insert_node(
                &mut graph,
                NodeId::new(1),
                Arc::from([1.0, 0.0, 0.0, 0.0]),
                0,
                &params,
            )
            .expect_err("duplicate rejected")
        }
        ApiInductionPayload::NonFiniteVector => {
            let config = HnswConfig::new(4).unwrap();
            let params = HnswParams::from_config(&config);
            insert_node(
                &mut HnswGraph::empty(4),
                NodeId::new(1),
                Arc::from([1.0, f32::NAN, 0.0, 0.0]),
                0,
                &params,
            )
            .expect_err("NaN rejected")
        }
        ApiInductionPayload::MaxLayerExceedsCap => {
            let config = HnswConfig::new(4).unwrap();
            let params = HnswParams::from_config(&config);
            insert_node(
                &mut HnswGraph::empty(4),
                NodeId::new(1),
                Arc::from([1.0, 0.0, 0.0, 0.0]),
                33,
                &params,
            )
            .expect_err("layer cap rejected")
        }
        ApiInductionPayload::NonFiniteQuery => provider
            .search(&[1.0, f32::NAN, 0.0, 0.0], 1, Some(4), None)
            .expect_err("NaN query rejected"),
        ApiInductionPayload::PqDimensionNotDivisible => {
            HnswConfig::with_params(10, 16, 200, 50, DistanceMetric::L2)
                .unwrap()
                .with_pq_quantization(PqParams {
                    m_subspaces: 3,
                    k_centroids: 256,
                    train_min_vectors: 256,
                })
                .expect_err("PQ dimension divisibility rejected")
        }
        _ => panic!("unknown API induction payload"),
    }
}

fn operation_not_supported_error(
    provider: &HnswProvider,
    payload: VectorUpsertPayloadV1,
) -> VectorError {
    // Codex review fix (P2): drive the error through the same code path
    // provider.on_change uses (apply_upsert), so the rendered fixture pins
    // the real VectorError shape (op / node_id / brief) instead of a
    // synthesized stand-in. Uses the test-harness-only typed accessor on
    // HnswProvider so we don't need to broaden builder's visibility.
    provider
        .apply_upsert_for_test(&payload)
        .err()
        .expect("reserved op rejected")
}

fn synthetic_error(fields: &SyntheticErrorFields) -> VectorError {
    match fields {
        SyntheticErrorFields::DimensionMismatch => VectorError::DimensionMismatch {
            expected: 4,
            observed: 3,
        },
        SyntheticErrorFields::SectionDecodeFailed => VectorError::SectionDecodeFailed {
            sub_tag: SubTag(*b"GRPH"),
            reason: "GRPH magic mismatch".into(),
        },
        SyntheticErrorFields::SectionEncodeFailed => VectorError::SectionEncodeFailed {
            sub_tag: SubTag(*b"VECS"),
            reason: "forced encode fixture".into(),
        },
        SyntheticErrorFields::EncodeFailed => VectorError::EncodeFailed {
            reason: "forced encode fixture".into(),
        },
        SyntheticErrorFields::InternalIndexExhausted => VectorError::InternalIndexExhausted {
            current: u32::MAX as usize + 1,
        },
        SyntheticErrorFields::PqTrainingDeferred => VectorError::PqTrainingDeferred {
            observed_vectors: 100,
            required: 256,
        },
        _ => panic!("unknown synthetic error fields"),
    }
}

pub(crate) fn canonical_error_for_kind(kind: VectorErrorKindMirror) -> VectorError {
    match kind {
        VectorErrorKindMirror::InvalidConfig => VectorError::InvalidConfig {
            reason: "dim must be greater than zero".into(),
        },
        VectorErrorKindMirror::DimensionMismatch => {
            synthetic_error(&SyntheticErrorFields::DimensionMismatch)
        }
        VectorErrorKindMirror::SectionDecodeFailed => {
            synthetic_error(&SyntheticErrorFields::SectionDecodeFailed)
        }
        VectorErrorKindMirror::SectionEncodeFailed => {
            synthetic_error(&SyntheticErrorFields::SectionEncodeFailed)
        }
        VectorErrorKindMirror::InvalidNodeId => VectorError::InvalidNodeId {
            node_id: NodeId::TOMBSTONE,
            reason: "NodeId::TOMBSTONE cannot be added to an HNSW index".into(),
        },
        VectorErrorKindMirror::DimensionsLocked => VectorError::DimensionsLocked {
            expected: 4,
            observed: 1,
        },
        VectorErrorKindMirror::InvalidPayload => VectorError::InvalidPayload {
            reason: "bulk-insert payload must contain at least one row".into(),
        },
        VectorErrorKindMirror::EncodeFailed => synthetic_error(&SyntheticErrorFields::EncodeFailed),
        VectorErrorKindMirror::OperationNotSupportedYet => VectorError::OperationNotSupportedYet {
            op: VectorOp::Update,
            node_id: NodeId::new(1),
            brief: "future",
        },
        VectorErrorKindMirror::DuplicateNodeId => VectorError::DuplicateNodeId {
            node_id: NodeId::new(1),
        },
        VectorErrorKindMirror::NonFiniteVectorComponent => VectorError::NonFiniteVectorComponent {
            node_id: NodeId::new(1),
            index: 1,
            value: f32::NAN,
        },
        VectorErrorKindMirror::InternalIndexExhausted => {
            synthetic_error(&SyntheticErrorFields::InternalIndexExhausted)
        }
        VectorErrorKindMirror::MaxLayerExceedsCap => VectorError::MaxLayerExceedsCap {
            observed: 33,
            cap: 32,
        },
        VectorErrorKindMirror::NonFiniteQueryComponent => VectorError::NonFiniteQueryComponent {
            index: 1,
            value: f32::NAN,
        },
        VectorErrorKindMirror::PqTrainingDeferred => {
            synthetic_error(&SyntheticErrorFields::PqTrainingDeferred)
        }
        VectorErrorKindMirror::PqDimensionNotDivisible => VectorError::PqDimensionNotDivisible {
            dim: 10,
            m_subspaces: 3,
        },
        _ => panic!("unknown vector error kind mirror"),
    }
}
