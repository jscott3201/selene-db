//! Deterministic renderer for the vector snapshot corpus.

use std::fmt;

use selene_core::NodeId;

use crate::snapshot::cqnt::PAYLOAD_MAGIC_CQNT;
use crate::snapshot::grph::PAYLOAD_MAGIC_GRPH;
use crate::snapshot::ipqb::PAYLOAD_MAGIC_IPQB;
use crate::snapshot::post::PAYLOAD_MAGIC_POST;
use crate::snapshot::qunt::PAYLOAD_MAGIC_QUNT;
use crate::snapshot::vecs::PAYLOAD_MAGIC_VECS;
use crate::{
    DistanceMetric, HnswConfig, HnswGraph, NeighborSelectionConfig, PAYLOAD_MAGIC,
    PAYLOAD_MAGIC_BULK, PAYLOAD_MAGIC_BULK_DELETE, PAYLOAD_MAGIC_IVF,
    PAYLOAD_MAGIC_IVF_BULK_DELETE, PAYLOAD_MAGIC_IVF_BULK_INSERT, PqParams, QuantMethod,
    VectorError, VectorOp,
};

pub mod errors;
pub mod quantization;
pub mod search;
pub mod sections;

pub use errors::render_vector_error;
pub use quantization::{QuantizationParitySummary, QuantizationStatsSummary};
pub use search::SearchRowsSummary;
pub use sections::VectorSectionsSummary;

/// Input accepted by [`vector_summary`].
#[derive(Clone, Debug)]
pub struct VectorSnapshotInput {
    /// Stable fixture slug.
    pub slug: String,
    /// Human-readable fixture description.
    pub description: String,
    /// Stable configuration summary.
    pub config: VectorConfigSummary,
    /// Stable graph summary.
    pub graph: VectorGraphSummary,
    /// Snapshot section bytes, when the fixture exercises sections.
    pub sections: Option<VectorSectionsSummary>,
    /// Invocation result rendered in the final block.
    pub invocation_result: VectorInvocationResult,
}

/// Stable provider configuration summary.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorConfigSummary {
    /// Vector dimensionality.
    pub dim: usize,
    /// HNSW neighbor bound.
    pub m: usize,
    /// Construction beam width.
    pub ef_construction: usize,
    /// Search beam width.
    pub ef_search: usize,
    /// Distance metric name.
    pub metric: &'static str,
    /// Whether SQ8 search is enabled.
    pub quantization_enabled: bool,
    /// Whether f32 rescore is enabled.
    pub quantization_rescore: bool,
    /// Quantization method rendered only when quantization is enabled.
    pub quantization_method: Option<&'static str>,
    /// PQ subspace count rendered only for PQ-enabled configs.
    pub pq_m: Option<usize>,
    /// PQ centroid count rendered only for PQ-enabled configs.
    pub pq_k: Option<u32>,
    /// PQ train-min threshold rendered only for PQ-enabled configs.
    pub pq_train_min: Option<usize>,
    /// Whether PQ configs train OPQ rotations.
    pub pq_opq: Option<bool>,
    /// Whether PQ configs train polysemous-code permutations.
    pub pq_polysemous: Option<bool>,
    /// Polysemous Hamming threshold ratio for PQ configs.
    pub pq_hamming_threshold_ratio: Option<f32>,
    /// HNSW neighbor-selection flavor.
    pub neighbor_select: &'static str,
}

impl VectorConfigSummary {
    /// Build a stable config summary from a provider config.
    #[must_use]
    pub fn from_config(config: &HnswConfig) -> Self {
        let pq_params = (config.quantization.enabled
            && config.quantization.method == QuantMethod::Pq)
            .then(|| PqParams::resolve(config.dim, config.quantization.pq));
        Self {
            dim: config.dim,
            m: config.m,
            ef_construction: config.ef_construction,
            ef_search: config.ef_search,
            metric: metric_name(config.metric),
            quantization_enabled: config.quantization.enabled,
            quantization_rescore: config.quantization.rescore,
            quantization_method: config
                .quantization
                .enabled
                .then(|| quant_method_token(config.quantization.method)),
            pq_m: pq_params.map(|params| params.m_subspaces),
            pq_k: pq_params.map(|params| params.k_centroids),
            pq_train_min: pq_params.map(|params| params.train_min_vectors),
            pq_opq: pq_params.map(|params| params.use_opq),
            pq_polysemous: pq_params.map(|params| params.use_polysemous),
            pq_hamming_threshold_ratio: pq_params.map(|params| params.hamming_threshold_ratio),
            neighbor_select: neighbor_selection_name(config.neighbor_selection),
        }
    }
}

/// Stable graph summary rendered before fixture results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorGraphSummary {
    /// Vector dimensionality.
    pub dimensions: usize,
    /// Number of indexed nodes.
    pub node_count: usize,
    /// Entry point, if any.
    pub entry_point: Option<u32>,
    /// Highest HNSW layer.
    pub max_layer: u8,
    /// Stable head sketch of the first indexed nodes.
    pub node_head: Vec<String>,
}

impl VectorGraphSummary {
    /// Build a stable graph summary from an immutable graph snapshot.
    #[must_use]
    pub fn from_graph(graph: &HnswGraph) -> Self {
        let node_head = graph
            .iter_nodes()
            .take(8)
            .map(|node| {
                let degrees = node
                    .neighbors
                    .iter()
                    .map(Vec::len)
                    .map(|degree| degree.to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                format!("n{}:L{}:[{}]", node.node_id.get(), node.max_layer, degrees)
            })
            .collect();
        Self {
            dimensions: graph.dimensions(),
            node_count: graph.len(),
            entry_point: graph.entry_point(),
            max_layer: graph.max_layer(),
            node_head,
        }
    }
}

/// Result shape produced by one vector snapshot fixture.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum VectorInvocationResult {
    /// Snapshot roundtrip result.
    Roundtrip {
        /// Whether recovered graph summary matched source.
        recovered_matches_source: bool,
    },
    /// Plain HNSW search rows.
    Search {
        /// Search rows sorted by score descending.
        rows: SearchRowsSummary,
    },
    /// Quantization parity rows.
    SearchParity {
        /// Parity summary across exact, SQ8, and rescored paths.
        parity: QuantizationParitySummary,
    },
    /// Quantization statistics.
    Stats {
        /// Renderable statistics.
        stats: QuantizationStatsSummary,
    },
    /// Snapshot recovery plus post-snapshot WAL replay.
    Replay {
        /// Post-replay graph summary.
        post_replay_summary: VectorGraphSummary,
        /// Whether recovered and from-scratch section bytes matched.
        byte_identical: bool,
    },
    /// Deliberate error fixture.
    Error {
        /// Stable error kind.
        kind: VectorErrorKind,
        /// `"api"` or `"synthetic"`.
        induction_kind: &'static str,
        /// Stable field projection.
        rendered: String,
    },
}

/// Rendered vector snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorSnapshot {
    /// Lines in display order.
    pub lines: Vec<String>,
}

impl fmt::Display for VectorSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            formatter.write_str(line)?;
        }
        Ok(())
    }
}

/// Build a stable textual summary for one corpus fixture.
#[must_use]
pub fn vector_summary(input: &VectorSnapshotInput) -> VectorSnapshot {
    let mut lines = Vec::new();
    lines.push("==== CONFIG ====".to_string());
    lines.push(format!("slug={}", quoted(&input.slug)));
    lines.push(format!("description={}", quoted(&input.description)));
    lines.push(format!(
        "dim={} m={} ef_construction={} ef_search={} metric={} quantization_enabled={} quantization_rescore={}",
        input.config.dim,
        input.config.m,
        input.config.ef_construction,
        input.config.ef_search,
        input.config.metric,
        input.config.quantization_enabled,
        input.config.quantization_rescore
    ));
    lines.push(format!(
        "neighbor_select={}",
        quoted(input.config.neighbor_select)
    ));
    if let Some(method) = input.config.quantization_method {
        match (
            input.config.pq_m,
            input.config.pq_k,
            input.config.pq_train_min,
            input.config.pq_opq,
            input.config.pq_polysemous,
            input.config.pq_hamming_threshold_ratio,
        ) {
            (
                Some(m),
                Some(k),
                Some(train_min),
                Some(opq),
                Some(polysemous),
                Some(hamming_threshold_ratio),
            ) => lines.push(format!(
                "quantization_method={} pq_m={m} pq_k={k} pq_train_min={train_min} pq_opq={opq} pq_polysemous={polysemous} pq_hamming_threshold_ratio={hamming_threshold_ratio:.3}",
                quoted(method),
            )),
            _ => lines.push(format!("quantization_method={}", quoted(method))),
        }
    }

    lines.push("==== GRAPH ====".to_string());
    lines.push(format!(
        "nodes={} dim={} entry_point={} max_layer={}",
        input.graph.node_count,
        input.graph.dimensions,
        input
            .graph
            .entry_point
            .map_or_else(|| "none".to_string(), |idx| format!("idx{idx}")),
        input.graph.max_layer
    ));
    lines.push(format!("node_head=[{}]", input.graph.node_head.join(", ")));

    lines.push("==== SECTIONS ====".to_string());
    match &input.sections {
        Some(sections) => sections::render_sections(sections, &mut lines),
        None => lines.push("<none>".to_string()),
    }

    lines.push("==== RESULT ====".to_string());
    match &input.invocation_result {
        VectorInvocationResult::Roundtrip {
            recovered_matches_source,
        } => lines.push(format!(
            "roundtrip.recovered_matches_source={recovered_matches_source}"
        )),
        VectorInvocationResult::Search { rows } => {
            search::render_search_rows("search", rows, &mut lines)
        }
        VectorInvocationResult::SearchParity { parity } => {
            quantization::render_parity(parity, &mut lines);
        }
        VectorInvocationResult::Stats { stats } => quantization::render_stats(stats, &mut lines),
        VectorInvocationResult::Replay {
            post_replay_summary,
            byte_identical,
        } => {
            lines.push(format!("replay.byte_identical={byte_identical}"));
            lines.push(format!(
                "replay.graph nodes={} dim={} entry_point={} max_layer={}",
                post_replay_summary.node_count,
                post_replay_summary.dimensions,
                post_replay_summary
                    .entry_point
                    .map_or_else(|| "none".to_string(), |idx| format!("idx{idx}")),
                post_replay_summary.max_layer
            ));
            lines.push(format!(
                "replay.node_head=[{}]",
                post_replay_summary.node_head.join(", ")
            ));
        }
        VectorInvocationResult::Error {
            kind,
            induction_kind,
            rendered,
        } => {
            lines.push(format!("error.kind={}", kind.name()));
            lines.push(format!("error.induction_kind={induction_kind}"));
            lines.push(format!("error.rendered={rendered}"));
        }
    }

    VectorSnapshot { lines }
}

/// Stable kind mirror local to `selene-vector`'s test-harness renderer.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VectorErrorKind {
    /// HNSW config validation failed.
    InvalidConfig,
    /// Low-level vector dimension mismatch.
    DimensionMismatch,
    /// Snapshot section decode failed.
    SectionDecodeFailed,
    /// Snapshot section encode failed.
    SectionEncodeFailed,
    /// Invalid source node ID.
    InvalidNodeId,
    /// Provider dimension lockout.
    DimensionsLocked,
    /// Invalid mutation payload.
    InvalidPayload,
    /// Mutation payload encode failed.
    EncodeFailed,
    /// Reserved operation.
    OperationNotSupportedYet,
    /// Duplicate vector node ID.
    DuplicateNodeId,
    /// Non-finite vector component.
    NonFiniteVectorComponent,
    /// InternalIndex exhaustion.
    InternalIndexExhausted,
    /// HNSW layer cap exceeded.
    MaxLayerExceedsCap,
    /// Non-finite query component.
    NonFiniteQueryComponent,
    /// Product quantization training is deferred.
    PqTrainingDeferred,
    /// Product quantization subspaces do not divide dimensions.
    PqDimensionNotDivisible,
    /// IVF-PQ training is deferred.
    IvfTrainingDeferred,
    /// IVF dimension mismatch.
    IvfDimensionMismatch,
    /// Invalid IVF probe count.
    IvfInvalidNProbe,
    /// IVF metric override requires side data absent from the index.
    IvfMetricOverrideRequiresSideData,
    /// IVF snapshot sections are inconsistent.
    IvfSectionInconsistent,
    /// IVF training failed.
    IvfTrainingFailed,
    /// PQ codebook training failed.
    PqCodebookTrainFailed,
    /// OPQ rotation training failed.
    OpqTrainingFailed,
    /// Polysemous-codes permutation training failed.
    PolysemousTrainingFailed,
}

impl VectorErrorKind {
    /// Stable variant name.
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
            Self::IvfMetricOverrideRequiresSideData => "IvfMetricOverrideRequiresSideData",
            Self::IvfSectionInconsistent => "IvfSectionInconsistent",
            Self::IvfTrainingFailed => "IvfTrainingFailed",
            Self::PqCodebookTrainFailed => "PqCodebookTrainFailed",
            Self::OpqTrainingFailed => "OpqTrainingFailed",
            Self::PolysemousTrainingFailed => "PolysemousTrainingFailed",
        }
    }
}

/// Anchor every `DistanceMetric` variant by name.
#[must_use]
pub fn distance_metric_anchor() -> &'static [(&'static str, DistanceMetric)] {
    &[
        ("Cosine", DistanceMetric::Cosine),
        ("L2", DistanceMetric::L2),
        ("Dot", DistanceMetric::Dot),
    ]
}

/// Anchor every `VectorOp` variant by name.
#[must_use]
pub fn vector_op_anchor() -> &'static [(&'static str, VectorOp)] {
    &[
        ("Insert", VectorOp::Insert),
        ("Update", VectorOp::Update),
        ("Delete", VectorOp::Delete),
    ]
}

/// Anchor every `QuantMethod` variant by name.
#[must_use]
pub fn quant_method_anchor() -> &'static [(&'static str, QuantMethod)] {
    &[("Sq8", QuantMethod::Sq8), ("Pq", QuantMethod::Pq)]
}

/// Anchor every neighbor-selection flavor by rendered token.
#[must_use]
pub fn neighbor_selection_anchor() -> &'static [(&'static str, NeighborSelectionConfig)] {
    &[
        (
            "default",
            NeighborSelectionConfig {
                extend_candidates: false,
                keep_pruned_connections: true,
            },
        ),
        (
            "extend_candidates",
            NeighborSelectionConfig {
                extend_candidates: true,
                keep_pruned_connections: true,
            },
        ),
        (
            "no_fill_back",
            NeighborSelectionConfig {
                extend_candidates: false,
                keep_pruned_connections: false,
            },
        ),
        (
            "extend_no_fill_back",
            NeighborSelectionConfig {
                extend_candidates: true,
                keep_pruned_connections: false,
            },
        ),
    ]
}

/// Expose all vector payload and section magic constants for drift tests.
#[must_use]
pub fn magic_constants() -> &'static [(&'static str, [u8; 4])] {
    &[
        ("VECU", PAYLOAD_MAGIC),
        ("VECB", PAYLOAD_MAGIC_BULK),
        ("VECD", PAYLOAD_MAGIC_BULK_DELETE),
        ("VGRP", PAYLOAD_MAGIC_GRPH),
        ("VVEC", PAYLOAD_MAGIC_VECS),
        ("VQNT", PAYLOAD_MAGIC_QUNT),
        ("VIVF", PAYLOAD_MAGIC_IVF),
        ("VIVB", PAYLOAD_MAGIC_IVF_BULK_INSERT),
        ("VIVD", PAYLOAD_MAGIC_IVF_BULK_DELETE),
        ("VCQB", PAYLOAD_MAGIC_CQNT),
        ("VIPB", PAYLOAD_MAGIC_IPQB),
        ("VPOS", PAYLOAD_MAGIC_POST),
    ]
}

/// Return the stable kind for a vector error.
#[must_use]
pub fn vector_error_kind_for(error: &VectorError) -> VectorErrorKind {
    match error {
        VectorError::InvalidConfig { .. } => VectorErrorKind::InvalidConfig,
        VectorError::DimensionMismatch { .. } => VectorErrorKind::DimensionMismatch,
        VectorError::SectionDecodeFailed { .. } => VectorErrorKind::SectionDecodeFailed,
        VectorError::SectionEncodeFailed { .. } => VectorErrorKind::SectionEncodeFailed,
        VectorError::InvalidNodeId { .. } => VectorErrorKind::InvalidNodeId,
        VectorError::DimensionsLocked { .. } => VectorErrorKind::DimensionsLocked,
        VectorError::InvalidPayload { .. } => VectorErrorKind::InvalidPayload,
        VectorError::EncodeFailed { .. } => VectorErrorKind::EncodeFailed,
        VectorError::OperationNotSupportedYet { .. } => VectorErrorKind::OperationNotSupportedYet,
        VectorError::DuplicateNodeId { .. } => VectorErrorKind::DuplicateNodeId,
        VectorError::NonFiniteVectorComponent { .. } => VectorErrorKind::NonFiniteVectorComponent,
        VectorError::InternalIndexExhausted { .. } => VectorErrorKind::InternalIndexExhausted,
        VectorError::MaxLayerExceedsCap { .. } => VectorErrorKind::MaxLayerExceedsCap,
        VectorError::NonFiniteQueryComponent { .. } => VectorErrorKind::NonFiniteQueryComponent,
        VectorError::PqTrainingDeferred { .. } => VectorErrorKind::PqTrainingDeferred,
        VectorError::PqDimensionNotDivisible { .. } => VectorErrorKind::PqDimensionNotDivisible,
        VectorError::IvfTrainingDeferred { .. } => VectorErrorKind::IvfTrainingDeferred,
        VectorError::IvfDimensionMismatch { .. } => VectorErrorKind::IvfDimensionMismatch,
        VectorError::IvfInvalidNProbe { .. } => VectorErrorKind::IvfInvalidNProbe,
        VectorError::IvfMetricOverrideRequiresSideData { .. } => {
            VectorErrorKind::IvfMetricOverrideRequiresSideData
        }
        VectorError::IvfSectionInconsistent { .. } => VectorErrorKind::IvfSectionInconsistent,
        VectorError::IvfTrainingFailed { .. } => VectorErrorKind::IvfTrainingFailed,
        VectorError::PqCodebookTrainFailed { .. } => VectorErrorKind::PqCodebookTrainFailed,
        VectorError::OpqTrainingFailed { .. } => VectorErrorKind::OpqTrainingFailed,
        VectorError::PolysemousTrainingFailed { .. } => VectorErrorKind::PolysemousTrainingFailed,
    }
}

/// Stable metric name for renderer and tests.
#[must_use]
pub const fn metric_name(metric: DistanceMetric) -> &'static str {
    match metric {
        DistanceMetric::Cosine => "Cosine",
        DistanceMetric::L2 => "L2",
        DistanceMetric::Dot => "Dot",
    }
}

/// Stable neighbor-selection flavor name.
#[must_use]
pub const fn neighbor_selection_name(config: NeighborSelectionConfig) -> &'static str {
    match (config.extend_candidates, config.keep_pruned_connections) {
        (false, true) => "default",
        (true, true) => "extend_candidates",
        (false, false) => "no_fill_back",
        (true, false) => "extend_no_fill_back",
    }
}

/// Stable operation name.
#[must_use]
pub const fn op_name(op: VectorOp) -> &'static str {
    match op {
        VectorOp::Insert => "Insert",
        VectorOp::Update => "Update",
        VectorOp::Delete => "Delete",
    }
}

/// Stable quantization method name.
#[must_use]
pub const fn quant_method_name(method: QuantMethod) -> &'static str {
    match method {
        QuantMethod::Sq8 => "Sq8",
        QuantMethod::Pq => "Pq",
    }
}

/// Stable quantization method token for config rendering.
#[must_use]
pub const fn quant_method_token(method: QuantMethod) -> &'static str {
    match method {
        QuantMethod::Sq8 => "sq8",
        QuantMethod::Pq => "pq",
    }
}

/// Format a node ID as the canonical snapshot token.
#[must_use]
pub fn render_node_id(node_id: NodeId) -> String {
    format!("n{}", node_id.get())
}

pub(crate) fn quoted(value: &str) -> String {
    format!("{value:?}")
}

pub(crate) fn format_score(score: f32) -> String {
    format!("{score:+.6}")
}
