//! Pure-mirror coverage tables for the selene-vector snapshot corpus.

#![allow(missing_docs)]

/// Public vector surfaces the BRIEF-64 corpus must exercise.
pub const SURFACE_COVERAGE: &[VectorSurface] = VectorSurface::ALL;

/// Distance metric variants the corpus mirrors from `selene-vector`.
pub const METRIC_COVERAGE: &[VectorMetricMirror] = VectorMetricMirror::ALL;

/// Vector operation variants the corpus mirrors from `selene-vector`.
pub const OP_COVERAGE: &[VectorOpMirror] = VectorOpMirror::ALL;

/// Vector error variants the corpus mirrors from `selene-vector`.
pub const ERROR_KIND_COVERAGE: &[VectorErrorKindMirror] = VectorErrorKindMirror::ALL;

/// Payload and snapshot-section magic constants the corpus mirrors.
pub const MAGIC_COVERAGE: &[VectorMagicMirror] = VectorMagicMirror::ALL;

/// Quantization methods the corpus mirrors from `selene-vector`.
pub const QUANT_METHOD_COVERAGE: &[QuantMethodMirror] = QuantMethodMirror::ALL;

/// HNSW neighbor-selection flavors the corpus mirrors from `selene-vector`.
pub const NEIGHBOR_SELECTION_COVERAGE: &[NeighborSelectionFlavor] = NeighborSelectionFlavor::ALL;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorSurface {
    HnswBuild,
    HnswSearchUnfiltered,
    HnswSearchFiltered,
    EventVecu,
    EventVecb,
    SnapshotGrph,
    SnapshotVecs,
    SnapshotQunt,
    QuantizationStatsAccessor,
    QuantizationAsymmetricSearch,
    QuantizationRescore,
    RecoveryReplay,
    IvfBuild,
    IvfSearchUnfiltered,
    IvfSearchFiltered,
    IvfRecovery,
    ErrorPath,
}

impl VectorSurface {
    pub const ALL: &'static [Self] = &[
        Self::HnswBuild,
        Self::HnswSearchUnfiltered,
        Self::HnswSearchFiltered,
        Self::EventVecu,
        Self::EventVecb,
        Self::SnapshotGrph,
        Self::SnapshotVecs,
        Self::SnapshotQunt,
        Self::QuantizationStatsAccessor,
        Self::QuantizationAsymmetricSearch,
        Self::QuantizationRescore,
        Self::RecoveryReplay,
        Self::IvfBuild,
        Self::IvfSearchUnfiltered,
        Self::IvfSearchFiltered,
        Self::IvfRecovery,
        Self::ErrorPath,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::HnswBuild => "HnswBuild",
            Self::HnswSearchUnfiltered => "HnswSearchUnfiltered",
            Self::HnswSearchFiltered => "HnswSearchFiltered",
            Self::EventVecu => "EventVecu",
            Self::EventVecb => "EventVecb",
            Self::SnapshotGrph => "SnapshotGrph",
            Self::SnapshotVecs => "SnapshotVecs",
            Self::SnapshotQunt => "SnapshotQunt",
            Self::QuantizationStatsAccessor => "QuantizationStatsAccessor",
            Self::QuantizationAsymmetricSearch => "QuantizationAsymmetricSearch",
            Self::QuantizationRescore => "QuantizationRescore",
            Self::RecoveryReplay => "RecoveryReplay",
            Self::IvfBuild => "IvfBuild",
            Self::IvfSearchUnfiltered => "IvfSearchUnfiltered",
            Self::IvfSearchFiltered => "IvfSearchFiltered",
            Self::IvfRecovery => "IvfRecovery",
            Self::ErrorPath => "ErrorPath",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorMetricMirror {
    Cosine,
    L2,
    Dot,
}

impl VectorMetricMirror {
    pub const ALL: &'static [Self] = &[Self::Cosine, Self::L2, Self::Dot];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cosine => "Cosine",
            Self::L2 => "L2",
            Self::Dot => "Dot",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorOpMirror {
    Insert,
    Update,
    Delete,
}

impl VectorOpMirror {
    pub const ALL: &'static [Self] = &[Self::Insert, Self::Update, Self::Delete];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Insert => "Insert",
            Self::Update => "Update",
            Self::Delete => "Delete",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorErrorKindMirror {
    InvalidConfig,
    DimensionMismatch,
    SectionDecodeFailed,
    SectionEncodeFailed,
    InvalidNodeId,
    DimensionsLocked,
    InvalidPayload,
    EncodeFailed,
    OperationNotSupportedYet,
    DuplicateNodeId,
    NonFiniteVectorComponent,
    InternalIndexExhausted,
    MaxLayerExceedsCap,
    NonFiniteQueryComponent,
    PqTrainingDeferred,
    PqDimensionNotDivisible,
    IvfTrainingDeferred,
    IvfDimensionMismatch,
    IvfInvalidNProbe,
    IvfSectionInconsistent,
    IvfTrainingFailed,
    PqCodebookTrainFailed,
    OpqTrainingFailed,
}

impl VectorErrorKindMirror {
    pub const ALL: &'static [Self] = &[
        Self::InvalidConfig,
        Self::DimensionMismatch,
        Self::SectionDecodeFailed,
        Self::SectionEncodeFailed,
        Self::InvalidNodeId,
        Self::DimensionsLocked,
        Self::InvalidPayload,
        Self::EncodeFailed,
        Self::OperationNotSupportedYet,
        Self::DuplicateNodeId,
        Self::NonFiniteVectorComponent,
        Self::InternalIndexExhausted,
        Self::MaxLayerExceedsCap,
        Self::NonFiniteQueryComponent,
        Self::PqTrainingDeferred,
        Self::PqDimensionNotDivisible,
        Self::IvfTrainingDeferred,
        Self::IvfDimensionMismatch,
        Self::IvfInvalidNProbe,
        Self::IvfSectionInconsistent,
        Self::IvfTrainingFailed,
        Self::PqCodebookTrainFailed,
        Self::OpqTrainingFailed,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::InvalidConfig => "InvalidConfig",
            Self::DimensionMismatch => "DimensionMismatch",
            Self::SectionDecodeFailed => "SectionDecodeFailed",
            Self::SectionEncodeFailed => "SectionEncodeFailed",
            Self::InvalidNodeId => "InvalidNodeId",
            Self::DimensionsLocked => "DimensionsLocked",
            Self::InvalidPayload => "InvalidPayload",
            Self::EncodeFailed => "EncodeFailed",
            Self::OperationNotSupportedYet => "OperationNotSupportedYet",
            Self::DuplicateNodeId => "DuplicateNodeId",
            Self::NonFiniteVectorComponent => "NonFiniteVectorComponent",
            Self::InternalIndexExhausted => "InternalIndexExhausted",
            Self::MaxLayerExceedsCap => "MaxLayerExceedsCap",
            Self::NonFiniteQueryComponent => "NonFiniteQueryComponent",
            Self::PqTrainingDeferred => "PqTrainingDeferred",
            Self::PqDimensionNotDivisible => "PqDimensionNotDivisible",
            Self::IvfTrainingDeferred => "IvfTrainingDeferred",
            Self::IvfDimensionMismatch => "IvfDimensionMismatch",
            Self::IvfInvalidNProbe => "IvfInvalidNProbe",
            Self::IvfSectionInconsistent => "IvfSectionInconsistent",
            Self::IvfTrainingFailed => "IvfTrainingFailed",
            Self::PqCodebookTrainFailed => "PqCodebookTrainFailed",
            Self::OpqTrainingFailed => "OpqTrainingFailed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VectorMagicMirror {
    Vecu,
    Vecb,
    Vecd,
    Vgrp,
    Vvec,
    Vqnt,
    Vivf,
    Vivb,
    Vivd,
    Vcqb,
    Vipb,
    Vpos,
}

impl VectorMagicMirror {
    pub const ALL: &'static [Self] = &[
        Self::Vecu,
        Self::Vecb,
        Self::Vecd,
        Self::Vgrp,
        Self::Vvec,
        Self::Vqnt,
        Self::Vivf,
        Self::Vivb,
        Self::Vivd,
        Self::Vcqb,
        Self::Vipb,
        Self::Vpos,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Vecu => "VECU",
            Self::Vecb => "VECB",
            Self::Vecd => "VECD",
            Self::Vgrp => "VGRP",
            Self::Vvec => "VVEC",
            Self::Vqnt => "VQNT",
            Self::Vivf => "VIVF",
            Self::Vivb => "VIVB",
            Self::Vivd => "VIVD",
            Self::Vcqb => "VCQB",
            Self::Vipb => "VIPB",
            Self::Vpos => "VPOS",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum QuantMethodMirror {
    Sq8,
    Pq,
}

impl QuantMethodMirror {
    pub const ALL: &'static [Self] = &[Self::Sq8, Self::Pq];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sq8 => "Sq8",
            Self::Pq => "Pq",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum NeighborSelectionFlavor {
    Default,
    ExtendCandidates,
    NoFillBack,
    ExtendNoFillBack,
}

impl NeighborSelectionFlavor {
    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::ExtendCandidates,
        Self::NoFillBack,
        Self::ExtendNoFillBack,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ExtendCandidates => "extend_candidates",
            Self::NoFillBack => "no_fill_back",
            Self::ExtendNoFillBack => "extend_no_fill_back",
        }
    }
}
