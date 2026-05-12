//! Curated selene-vector snapshot corpus fixtures (D21 pure-mirror DSL).

#![allow(missing_docs)]

pub mod coverage;
pub mod fixtures;

pub use coverage::{
    ERROR_KIND_COVERAGE, MAGIC_COVERAGE, METRIC_COVERAGE, NEIGHBOR_SELECTION_COVERAGE,
    NeighborSelectionFlavor, OP_COVERAGE, QUANT_METHOD_COVERAGE, QuantMethodMirror,
    SURFACE_COVERAGE, VectorErrorKindMirror, VectorMagicMirror, VectorMetricMirror, VectorOpMirror,
    VectorSurface,
};
pub use fixtures::{
    ApiInductionPayload, ErrorInductionKind, SyntheticErrorFields, VectorConfigSpec,
    VectorCorpusEvent, VectorCorpusGraph, VectorCorpusInvocation, VectorQuantizationSpec,
};

/// Curated corpus of vector snapshot entries.
#[derive(Clone, Debug)]
pub struct VectorCorpus {
    entries: Vec<VectorCorpusEntry>,
}

/// One entry in the vector snapshot corpus.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VectorCorpusEntry {
    /// Snake-case slug used as the snapshot file suffix.
    pub slug: &'static str,
    /// Short description surfaced in test failures and snapshots.
    pub description: &'static str,
    /// Graph fixture used to build the provider state.
    pub graph: VectorCorpusGraph,
    /// Provider configuration mirror.
    pub config: VectorConfigSpec,
    /// Invocation executed after the graph fixture is built.
    pub invocation: VectorCorpusInvocation,
    /// Coarse corpus category.
    pub category: VectorCorpusCategory,
    /// Public surfaces covered by this entry.
    pub covered_surfaces: &'static [VectorSurface],
    /// Distance metrics covered by this entry.
    pub covered_metrics: &'static [VectorMetricMirror],
    /// Vector operations covered by this entry.
    pub covered_ops: &'static [VectorOpMirror],
    /// Vector error variants covered by this entry.
    pub covered_errors: &'static [VectorErrorKindMirror],
    /// Payload/section magics covered by this entry.
    pub covered_magics: &'static [VectorMagicMirror],
    /// Quantization methods covered by this entry.
    pub covered_quant_methods: &'static [QuantMethodMirror],
}

/// Coarse-grained vector corpus bucket.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorCorpusCategory {
    /// Snapshot section encoding and recovery.
    Snapshot,
    /// Build/search behavior.
    Search,
    /// Post-snapshot WAL replay.
    Recovery,
    /// SQ8 quantization and stats.
    Quantization,
    /// Deliberate error rendering.
    Error,
}

impl VectorCorpusCategory {
    /// All category variants in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Snapshot,
        Self::Search,
        Self::Recovery,
        Self::Quantization,
        Self::Error,
    ];
}

impl VectorCorpus {
    /// The curated M8 closure corpus.
    #[must_use]
    pub fn m8() -> Self {
        Self { entries: entries() }
    }

    /// Iterate entries in declaration order.
    pub fn entries(&self) -> impl Iterator<Item = &VectorCorpusEntry> + '_ {
        self.entries.iter()
    }

    /// Number of entries in this corpus.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the corpus is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn cfg(
    dim: usize,
    metric: VectorMetricMirror,
    quantization: VectorQuantizationSpec,
) -> VectorConfigSpec {
    VectorConfigSpec::new(dim, metric, quantization)
}

fn cfg_neighbor(
    dim: usize,
    metric: VectorMetricMirror,
    quantization: VectorQuantizationSpec,
    neighbor_selection_flavor: NeighborSelectionFlavor,
) -> VectorConfigSpec {
    VectorConfigSpec::new(dim, metric, quantization)
        .with_neighbor_selection(neighbor_selection_flavor)
}

fn cfg_neighbor_hnsw(
    dim: usize,
    metric: VectorMetricMirror,
    quantization: VectorQuantizationSpec,
    neighbor_selection_flavor: NeighborSelectionFlavor,
    m: usize,
    ef_construction: usize,
    ef_search: usize,
) -> VectorConfigSpec {
    cfg_neighbor(dim, metric, quantization, neighbor_selection_flavor).with_hnsw(
        m,
        ef_construction,
        ef_search,
    )
}

fn search(
    query: &[f32],
    k: usize,
    ef_search: Option<usize>,
    filter: Option<Vec<u64>>,
) -> VectorCorpusInvocation {
    VectorCorpusInvocation::Search {
        query: query.to_vec(),
        k,
        ef_search,
        filter,
    }
}

fn insert(node_id_raw: u64, vector: &[f32], max_layer: u8) -> VectorCorpusEvent {
    VectorCorpusEvent::Insert {
        node_id_raw,
        vector: vector.to_vec(),
        max_layer,
    }
}

fn bulk(rows: &[(u64, &[f32], u8)]) -> VectorCorpusEvent {
    VectorCorpusEvent::Bulk {
        rows: rows
            .iter()
            .map(|(raw, vector, layer)| (*raw, vector.to_vec(), *layer))
            .collect(),
    }
}

fn api_error(kind: VectorErrorKindMirror, payload: ApiInductionPayload) -> VectorCorpusInvocation {
    VectorCorpusInvocation::DeliberateApiError { kind, payload }
}

fn synthetic_error(
    kind: VectorErrorKindMirror,
    fields: SyntheticErrorFields,
) -> VectorCorpusInvocation {
    VectorCorpusInvocation::DeliberateSyntheticError { kind, fields }
}

#[allow(clippy::too_many_lines)]
fn entries() -> Vec<VectorCorpusEntry> {
    use NeighborSelectionFlavor as N;
    use QuantMethodMirror as Q;
    use VectorCorpusCategory as C;
    use VectorCorpusGraph as G;
    use VectorErrorKindMirror as E;
    use VectorMagicMirror as X;
    use VectorMetricMirror as M;
    use VectorOpMirror as O;
    use VectorQuantizationSpec as Z;
    use VectorSurface as S;

    vec![
        VectorCorpusEntry {
            slug: "empty-snapshot-roundtrip",
            description: "Empty graph GRPH/VECS roundtrip.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: VectorCorpusInvocation::SnapshotRoundtrip,
            category: C::Snapshot,
            covered_surfaces: &[S::SnapshotGrph, S::SnapshotVecs],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "single-node-search-cosine",
            description: "One VECU insert and one cosine search.",
            graph: G::SingleOriginCosine,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: search(&[1.0, 0.0, 0.0, 0.0], 1, Some(4), None),
            category: C::Search,
            covered_surfaces: &[S::HnswBuild, S::EventVecu, S::HnswSearchUnfiltered],
            covered_metrics: &[M::Cosine],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecu],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "orthogonal-basis-search-cosine",
            description: "Cosine top-k over four orthogonal basis vectors.",
            graph: G::OrthogonalBasisCosine4,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: search(&[1.0, 0.0, 0.0, 0.0], 3, Some(8), None),
            category: C::Search,
            covered_surfaces: &[
                S::HnswBuild,
                S::HnswSearchUnfiltered,
                S::SnapshotGrph,
                S::SnapshotVecs,
            ],
            covered_metrics: &[M::Cosine],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecu, X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "orthogonal-basis-search-l2",
            description: "L2 search over four orthogonal basis vectors.",
            graph: G::OrthogonalBasisCosine4,
            config: cfg(4, M::L2, Z::DISABLED),
            invocation: search(&[1.0, 0.0, 0.0, 0.0], 3, Some(8), None),
            category: C::Search,
            covered_surfaces: &[S::HnswSearchUnfiltered],
            covered_metrics: &[M::L2],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "orthogonal-basis-search-dot",
            description: "Dot-product search over four orthogonal basis vectors.",
            graph: G::OrthogonalBasisCosine4,
            config: cfg(4, M::Dot, Z::DISABLED),
            invocation: search(&[1.0, 0.0, 0.0, 0.0], 3, Some(8), None),
            category: C::Search,
            covered_surfaces: &[S::HnswSearchUnfiltered],
            covered_metrics: &[M::Dot],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "bulk-100-search-cosine",
            description: "One VECB bulk event and cosine search over 100 vectors.",
            graph: G::DeterministicL2_100,
            config: cfg(8, M::Cosine, Z::DISABLED),
            invocation: search(
                &[0.8, -0.3, 0.2, 0.1, -0.4, 0.7, -0.2, 0.5],
                8,
                Some(100),
                None,
            ),
            category: C::Search,
            covered_surfaces: &[S::EventVecb, S::HnswBuild, S::HnswSearchUnfiltered],
            covered_metrics: &[M::Cosine],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecb],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "mixed-layer-30-search-cosine",
            description: "Multi-layer graph exercises greedy descent.",
            graph: G::MixedLayerCosine30,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: search(&[0.25, -0.5, 0.75, 0.125], 5, Some(30), None),
            category: C::Search,
            covered_surfaces: &[S::HnswBuild, S::HnswSearchUnfiltered],
            covered_metrics: &[M::Cosine],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecu],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "filtered-search-cosine",
            description: "RoaringBitmap result-admission filter over raw NodeId values.",
            graph: G::MixedLayerCosine30,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: search(
                &[0.25, -0.5, 0.75, 0.125],
                5,
                Some(30),
                Some(vec![2, 4, 6, 8, 10, 12]),
            ),
            category: C::Search,
            covered_surfaces: &[S::HnswSearchFiltered],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "diverse-cluster-extend-off",
            description: "Diverse L2 cluster search with default neighbor selection.",
            graph: G::DiverseClusterL2_64,
            config: cfg_neighbor_hnsw(8, M::L2, Z::DISABLED, N::Default, 8, 64, 50),
            invocation: search(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 8, Some(50), None),
            category: C::Search,
            covered_surfaces: &[S::HnswBuild, S::HnswSearchUnfiltered],
            covered_metrics: &[M::L2],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecb, X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "diverse-cluster-extend-on",
            description: "Diverse L2 cluster search with one-hop candidate extension.",
            graph: G::DiverseClusterL2_64,
            config: cfg_neighbor_hnsw(8, M::L2, Z::DISABLED, N::ExtendCandidates, 8, 64, 50),
            invocation: search(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 8, Some(50), None),
            category: C::Search,
            covered_surfaces: &[S::HnswBuild, S::HnswSearchUnfiltered],
            covered_metrics: &[M::L2],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecb, X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "pruned-fill-disabled",
            description: "Dense L2 cluster demonstrates no-fill-back sub-degree selection.",
            graph: G::DenseClusterL2_8,
            config: cfg_neighbor_hnsw(8, M::L2, Z::DISABLED, N::NoFillBack, 4, 8, 16),
            invocation: VectorCorpusInvocation::SnapshotRoundtrip,
            category: C::Snapshot,
            covered_surfaces: &[S::HnswBuild, S::SnapshotGrph, S::SnapshotVecs],
            covered_metrics: &[M::L2],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecb, X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "snapshot-roundtrip-30",
            description: "GRPH/VECS roundtrip for a 30-node mixed-layer graph.",
            graph: G::MixedLayerCosine30,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: VectorCorpusInvocation::SnapshotRoundtrip,
            category: C::Snapshot,
            covered_surfaces: &[S::SnapshotGrph, S::SnapshotVecs, S::RecoveryReplay],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "recovery-replay-30-plus-5-vecu",
            description: "Snapshot recovery followed by five VECU events.",
            graph: G::MixedLayerCosine30,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: VectorCorpusInvocation::RecoveryReplay {
                post_snapshot_events: vec![
                    insert(31, &[0.1, 0.2, 0.3, 0.4], 0),
                    insert(32, &[0.4, 0.3, 0.2, 0.1], 0),
                    insert(33, &[-0.3, 0.1, 0.5, -0.2], 1),
                    insert(34, &[0.7, -0.4, 0.0, 0.2], 0),
                    insert(35, &[-0.1, -0.2, 0.6, 0.3], 0),
                ],
            },
            category: C::Recovery,
            covered_surfaces: &[S::RecoveryReplay, S::EventVecu],
            covered_metrics: &[M::Cosine],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecu, X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "recovery-replay-30-plus-5-vecb",
            description: "Snapshot recovery followed by one VECB event containing five rows.",
            graph: G::MixedLayerCosine30,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: VectorCorpusInvocation::RecoveryReplay {
                post_snapshot_events: vec![bulk(&[
                    (31, &[0.1, 0.2, 0.3, 0.4], 0),
                    (32, &[0.4, 0.3, 0.2, 0.1], 0),
                    (33, &[-0.3, 0.1, 0.5, -0.2], 1),
                    (34, &[0.7, -0.4, 0.0, 0.2], 0),
                    (35, &[-0.1, -0.2, 0.6, 0.3], 0),
                ])],
            },
            category: C::Recovery,
            covered_surfaces: &[S::RecoveryReplay, S::EventVecb],
            covered_metrics: &[M::Cosine],
            covered_ops: &[O::Insert],
            covered_errors: &[],
            covered_magics: &[X::Vecb, X::Vgrp, X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "quantized-prefix-stats",
            description: "QUNT roundtrip with QuantizationStats rendering.",
            graph: G::QuantizedPrefixCosine,
            config: cfg(4, M::Cosine, Z::ENABLED),
            invocation: VectorCorpusInvocation::StatsOnly,
            category: C::Quantization,
            covered_surfaces: &[S::SnapshotQunt, S::QuantizationStatsAccessor],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vqnt],
            covered_quant_methods: &[Q::Sq8],
        },
        VectorCorpusEntry {
            slug: "quantized-asymmetric-search-cosine",
            description: "SQ8 asymmetric cosine search with f32 baseline rendered beside it.",
            graph: G::QuantizedPrefixCosine,
            config: cfg(4, M::Cosine, Z::ENABLED),
            invocation: search(&[0.3, -0.2, 0.7, 0.1], 6, Some(50), None),
            category: C::Quantization,
            covered_surfaces: &[S::QuantizationAsymmetricSearch, S::SnapshotQunt],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vqnt],
            covered_quant_methods: &[Q::Sq8],
        },
        VectorCorpusEntry {
            slug: "quantized-rescore-search-cosine",
            description: "SQ8 asymmetric cosine search with f32 rescore enabled.",
            graph: G::QuantizedPrefixCosine,
            config: cfg(4, M::Cosine, Z::RESCORE),
            invocation: search(&[0.3, -0.2, 0.7, 0.1], 6, Some(50), None),
            category: C::Quantization,
            covered_surfaces: &[S::QuantizationRescore, S::QuantizationAsymmetricSearch],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vqnt],
            covered_quant_methods: &[Q::Sq8],
        },
        VectorCorpusEntry {
            slug: "quantized-asymmetric-search-l2",
            description: "SQ8 asymmetric L2 search on the same numeric scale as f32.",
            graph: G::QuantizedPrefixCosine,
            config: cfg(4, M::L2, Z::ENABLED),
            invocation: search(&[0.3, -0.2, 0.7, 0.1], 6, Some(50), None),
            category: C::Quantization,
            covered_surfaces: &[S::QuantizationAsymmetricSearch],
            covered_metrics: &[M::L2],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vqnt],
            covered_quant_methods: &[Q::Sq8],
        },
        VectorCorpusEntry {
            slug: "quantized-asymmetric-search-dot",
            description: "SQ8 asymmetric dot search on the same numeric scale as f32.",
            graph: G::QuantizedPrefixCosine,
            config: cfg(4, M::Dot, Z::ENABLED),
            invocation: search(&[0.3, -0.2, 0.7, 0.1], 6, Some(50), None),
            category: C::Quantization,
            covered_surfaces: &[S::QuantizationAsymmetricSearch],
            covered_metrics: &[M::Dot],
            covered_ops: &[],
            covered_errors: &[],
            covered_magics: &[X::Vqnt],
            covered_quant_methods: &[Q::Sq8],
        },
        VectorCorpusEntry {
            slug: "error-invalid-config",
            description: "InvalidConfig from zero-dimensional HNSW config.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(E::InvalidConfig, ApiInductionPayload::InvalidConfigZeroDim),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::InvalidConfig],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-dimension-mismatch",
            description: "Synthetic DimensionMismatch shape.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: synthetic_error(
                E::DimensionMismatch,
                SyntheticErrorFields::DimensionMismatch,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::DimensionMismatch],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-section-decode-failed",
            description: "Synthetic SectionDecodeFailed shape.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: synthetic_error(
                E::SectionDecodeFailed,
                SyntheticErrorFields::SectionDecodeFailed,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::SectionDecodeFailed],
            covered_magics: &[X::Vgrp],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-section-encode-failed",
            description: "Synthetic SectionEncodeFailed shape.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: synthetic_error(
                E::SectionEncodeFailed,
                SyntheticErrorFields::SectionEncodeFailed,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::SectionEncodeFailed],
            covered_magics: &[X::Vvec],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-invalid-node-id",
            description: "InvalidNodeId from NodeId::TOMBSTONE.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::InvalidNodeId,
                ApiInductionPayload::InvalidNodeIdTombstone,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[O::Insert],
            covered_errors: &[E::InvalidNodeId],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-dimensions-locked",
            description: "DimensionsLocked from wrong-dimensional search.",
            graph: G::SingleOriginCosine,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::DimensionsLocked,
                ApiInductionPayload::DimensionsLockedSearch,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[E::DimensionsLocked],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-invalid-payload",
            description: "InvalidPayload from empty bulk payload.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::InvalidPayload,
                ApiInductionPayload::InvalidPayloadEmptyBulk,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::InvalidPayload],
            covered_magics: &[X::Vecb],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-encode-failed",
            description: "Synthetic EncodeFailed shape.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: synthetic_error(E::EncodeFailed, SyntheticErrorFields::EncodeFailed),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::EncodeFailed],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-operation-update-not-supported",
            description: "OperationNotSupportedYet from reserved Update op.",
            graph: G::SingleOriginCosine,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::OperationNotSupportedYet,
                ApiInductionPayload::OperationUpdate,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[O::Update],
            covered_errors: &[E::OperationNotSupportedYet],
            covered_magics: &[X::Vecu],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-operation-delete-not-supported",
            description: "OperationNotSupportedYet from reserved Delete op.",
            graph: G::SingleOriginCosine,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::OperationNotSupportedYet,
                ApiInductionPayload::OperationDelete,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[O::Delete],
            covered_errors: &[E::OperationNotSupportedYet],
            covered_magics: &[X::Vecu],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-duplicate-node-id",
            description: "DuplicateNodeId from re-inserting an existing node.",
            graph: G::SingleOriginCosine,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(E::DuplicateNodeId, ApiInductionPayload::DuplicateNodeId),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[O::Insert],
            covered_errors: &[E::DuplicateNodeId],
            covered_magics: &[X::Vecu],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-non-finite-vector",
            description: "NonFiniteVectorComponent from a NaN vector component.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::NonFiniteVectorComponent,
                ApiInductionPayload::NonFiniteVector,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[O::Insert],
            covered_errors: &[E::NonFiniteVectorComponent],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-internal-index-exhausted",
            description: "Synthetic InternalIndexExhausted shape.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: synthetic_error(
                E::InternalIndexExhausted,
                SyntheticErrorFields::InternalIndexExhausted,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[],
            covered_errors: &[E::InternalIndexExhausted],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-max-layer-exceeds-cap",
            description: "MaxLayerExceedsCap from a direct HNSW insertion.",
            graph: G::Empty,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::MaxLayerExceedsCap,
                ApiInductionPayload::MaxLayerExceedsCap,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[],
            covered_ops: &[O::Insert],
            covered_errors: &[E::MaxLayerExceedsCap],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
        VectorCorpusEntry {
            slug: "error-non-finite-query",
            description: "NonFiniteQueryComponent from a NaN query component.",
            graph: G::SingleOriginCosine,
            config: cfg(4, M::Cosine, Z::DISABLED),
            invocation: api_error(
                E::NonFiniteQueryComponent,
                ApiInductionPayload::NonFiniteQuery,
            ),
            category: C::Error,
            covered_surfaces: &[S::ErrorPath],
            covered_metrics: &[M::Cosine],
            covered_ops: &[],
            covered_errors: &[E::NonFiniteQueryComponent],
            covered_magics: &[],
            covered_quant_methods: &[],
        },
    ]
}
