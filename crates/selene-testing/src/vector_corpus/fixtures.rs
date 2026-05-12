//! Fixture vocabulary for the selene-vector snapshot corpus.

#![allow(missing_docs)]

use super::coverage::{NeighborSelectionFlavor, VectorErrorKindMirror, VectorMetricMirror};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorCorpusGraph {
    Empty,
    SingleOriginCosine,
    OrthogonalBasisCosine4,
    DeterministicL2_100,
    MixedLayerCosine30,
    QuantizedPrefixCosine,
    DiverseClusterL2_64,
    DenseClusterL2_8,
}

impl VectorCorpusGraph {
    pub const ALL: &'static [Self] = &[
        Self::Empty,
        Self::SingleOriginCosine,
        Self::OrthogonalBasisCosine4,
        Self::DeterministicL2_100,
        Self::MixedLayerCosine30,
        Self::QuantizedPrefixCosine,
        Self::DiverseClusterL2_64,
        Self::DenseClusterL2_8,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::SingleOriginCosine => "single-origin-cosine",
            Self::OrthogonalBasisCosine4 => "orthogonal-basis-cosine-4",
            Self::DeterministicL2_100 => "deterministic-l2-100",
            Self::MixedLayerCosine30 => "mixed-layer-cosine-30",
            Self::QuantizedPrefixCosine => "quantized-prefix-cosine",
            Self::DiverseClusterL2_64 => "diverse-cluster-l2-64",
            Self::DenseClusterL2_8 => "dense-cluster-l2-8",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct VectorQuantizationSpec {
    pub enabled: bool,
    pub rescore: bool,
}

impl VectorQuantizationSpec {
    pub const DISABLED: Self = Self {
        enabled: false,
        rescore: false,
    };

    pub const ENABLED: Self = Self {
        enabled: true,
        rescore: false,
    };

    pub const RESCORE: Self = Self {
        enabled: true,
        rescore: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct VectorConfigSpec {
    pub dim: usize,
    pub m: usize,
    pub ef_construction: usize,
    pub ef_search: usize,
    pub metric: VectorMetricMirror,
    pub quantization: VectorQuantizationSpec,
    pub neighbor_selection_flavor: NeighborSelectionFlavor,
}

impl VectorConfigSpec {
    #[must_use]
    pub const fn new(
        dim: usize,
        metric: VectorMetricMirror,
        quantization: VectorQuantizationSpec,
    ) -> Self {
        Self {
            dim,
            m: 16,
            ef_construction: 200,
            ef_search: if dim > 50 { dim } else { 50 },
            metric,
            quantization,
            neighbor_selection_flavor: NeighborSelectionFlavor::Default,
        }
    }

    #[must_use]
    pub const fn with_hnsw(mut self, m: usize, ef_construction: usize, ef_search: usize) -> Self {
        self.m = m;
        self.ef_construction = ef_construction;
        self.ef_search = ef_search;
        self
    }

    #[must_use]
    pub const fn with_neighbor_selection(
        mut self,
        neighbor_selection_flavor: NeighborSelectionFlavor,
    ) -> Self {
        self.neighbor_selection_flavor = neighbor_selection_flavor;
        self
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum VectorCorpusEvent {
    Insert {
        node_id_raw: u64,
        vector: Vec<f32>,
        max_layer: u8,
    },
    Bulk {
        rows: Vec<(u64, Vec<f32>, u8)>,
    },
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum VectorCorpusInvocation {
    SnapshotRoundtrip,
    Search {
        query: Vec<f32>,
        k: usize,
        ef_search: Option<usize>,
        filter: Option<Vec<u64>>,
    },
    RecoveryReplay {
        post_snapshot_events: Vec<VectorCorpusEvent>,
    },
    StatsOnly,
    DeliberateApiError {
        kind: VectorErrorKindMirror,
        payload: ApiInductionPayload,
    },
    DeliberateSyntheticError {
        kind: VectorErrorKindMirror,
        fields: SyntheticErrorFields,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ErrorInductionKind {
    Api,
    Synthetic,
}

impl ErrorInductionKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Synthetic => "synthetic",
        }
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ApiInductionPayload {
    InvalidConfigZeroDim,
    InvalidNodeIdTombstone,
    DimensionsLockedSearch,
    InvalidPayloadEmptyBulk,
    OperationUpdate,
    OperationDelete,
    DuplicateNodeId,
    NonFiniteVector,
    MaxLayerExceedsCap,
    NonFiniteQuery,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SyntheticErrorFields {
    DimensionMismatch,
    SectionDecodeFailed,
    SectionEncodeFailed,
    EncodeFailed,
    InternalIndexExhausted,
}

impl VectorCorpusInvocation {
    #[must_use]
    pub const fn induction_kind(&self) -> Option<ErrorInductionKind> {
        match self {
            Self::DeliberateApiError { .. } => Some(ErrorInductionKind::Api),
            Self::DeliberateSyntheticError { .. } => Some(ErrorInductionKind::Synthetic),
            Self::SnapshotRoundtrip
            | Self::Search { .. }
            | Self::RecoveryReplay { .. }
            | Self::StatsOnly => None,
        }
    }
}
