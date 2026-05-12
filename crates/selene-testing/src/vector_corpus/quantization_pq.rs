//! Product-quantization corpus entries.

use super::coverage::{
    QuantMethodMirror, VectorErrorKindMirror, VectorMagicMirror, VectorMetricMirror,
    VectorOpMirror, VectorSurface,
};
use super::fixtures::{VectorCorpusGraph, VectorCorpusInvocation, VectorQuantizationSpec};
use super::{VectorCorpusCategory, VectorCorpusEntry, cfg, insert, search};

const SEARCH_SURFACES: &[VectorSurface] = &[
    VectorSurface::HnswBuild,
    VectorSurface::QuantizationAsymmetricSearch,
    VectorSurface::SnapshotQunt,
];
const RESCORE_SURFACES: &[VectorSurface] = &[
    VectorSurface::QuantizationAsymmetricSearch,
    VectorSurface::QuantizationRescore,
    VectorSurface::SnapshotQunt,
];
const REPLAY_SURFACES: &[VectorSurface] = &[
    VectorSurface::RecoveryReplay,
    VectorSurface::SnapshotQunt,
    VectorSurface::EventVecu,
];
const L2_METRIC: &[VectorMetricMirror] = &[VectorMetricMirror::L2];
const INSERT_OP: &[VectorOpMirror] = &[VectorOpMirror::Insert];
const QUNT_MAGIC: &[VectorMagicMirror] = &[VectorMagicMirror::Vqnt];
const NO_ERRORS: &[VectorErrorKindMirror] = &[];
const PQ_METHOD: &[QuantMethodMirror] = &[QuantMethodMirror::Pq];

pub(crate) fn default_l2_entry() -> VectorCorpusEntry {
    VectorCorpusEntry {
        slug: "pq-default-l2",
        description: "PQ asymmetric L2 search with trained QUNT codebook.",
        graph: VectorCorpusGraph::PqTrainingL2_256,
        config: cfg(
            8,
            VectorMetricMirror::L2,
            VectorQuantizationSpec::PQ_DEFAULT,
        ),
        invocation: search(
            &[0.2, -0.4, 0.1, 0.8, -0.2, 0.3, -0.7, 0.5],
            8,
            Some(80),
            None,
        ),
        category: VectorCorpusCategory::Quantization,
        covered_surfaces: SEARCH_SURFACES,
        covered_metrics: L2_METRIC,
        covered_ops: INSERT_OP,
        covered_errors: NO_ERRORS,
        covered_magics: QUNT_MAGIC,
        covered_quant_methods: PQ_METHOD,
    }
}

pub(crate) fn rescore_l2_entry() -> VectorCorpusEntry {
    VectorCorpusEntry {
        slug: "pq-rescore-l2",
        description: "PQ asymmetric L2 search with exact f32 rescore.",
        graph: VectorCorpusGraph::PqTrainingL2_256,
        config: cfg(
            8,
            VectorMetricMirror::L2,
            VectorQuantizationSpec::PQ_RESCORE,
        ),
        invocation: search(
            &[0.2, -0.4, 0.1, 0.8, -0.2, 0.3, -0.7, 0.5],
            8,
            Some(80),
            None,
        ),
        category: VectorCorpusCategory::Quantization,
        covered_surfaces: RESCORE_SURFACES,
        covered_metrics: L2_METRIC,
        covered_ops: INSERT_OP,
        covered_errors: NO_ERRORS,
        covered_magics: QUNT_MAGIC,
        covered_quant_methods: PQ_METHOD,
    }
}

pub(crate) fn recovery_replay_entry() -> VectorCorpusEntry {
    VectorCorpusEntry {
        slug: "pq-recovery-replay",
        description: "PQ snapshot recovery followed by five f32 WAL inserts.",
        graph: VectorCorpusGraph::PqTrainingL2_256,
        config: cfg(
            8,
            VectorMetricMirror::L2,
            VectorQuantizationSpec::PQ_DEFAULT,
        ),
        invocation: VectorCorpusInvocation::RecoveryReplay {
            post_snapshot_events: vec![
                insert(301, &[0.1, 0.2, 0.3, 0.4, -0.1, -0.2, 0.5, 0.6], 0),
                insert(302, &[0.2, 0.1, 0.4, 0.3, -0.2, -0.1, 0.6, 0.5], 0),
                insert(303, &[-0.3, 0.1, 0.5, -0.2, 0.7, -0.4, 0.0, 0.2], 1),
                insert(304, &[0.7, -0.4, 0.0, 0.2, -0.3, 0.1, 0.5, -0.2], 0),
                insert(305, &[-0.1, -0.2, 0.6, 0.3, 0.1, 0.2, 0.3, 0.4], 0),
            ],
        },
        category: VectorCorpusCategory::Recovery,
        covered_surfaces: REPLAY_SURFACES,
        covered_metrics: L2_METRIC,
        covered_ops: INSERT_OP,
        covered_errors: NO_ERRORS,
        covered_magics: QUNT_MAGIC,
        covered_quant_methods: PQ_METHOD,
    }
}

pub(crate) fn training_deferred_entry() -> VectorCorpusEntry {
    VectorCorpusEntry {
        slug: "pq-training-deferred",
        description: "PQ below train_min writes empty QUNT and searches with f32 fallback.",
        graph: VectorCorpusGraph::DeterministicL2_100,
        config: cfg(
            8,
            VectorMetricMirror::L2,
            VectorQuantizationSpec::PQ_DEFERRED,
        ),
        invocation: search(
            &[0.2, -0.4, 0.1, 0.8, -0.2, 0.3, -0.7, 0.5],
            8,
            Some(80),
            None,
        ),
        category: VectorCorpusCategory::Quantization,
        covered_surfaces: SEARCH_SURFACES,
        covered_metrics: L2_METRIC,
        covered_ops: INSERT_OP,
        covered_errors: NO_ERRORS,
        covered_magics: QUNT_MAGIC,
        covered_quant_methods: PQ_METHOD,
    }
}
