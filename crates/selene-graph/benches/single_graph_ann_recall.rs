#![allow(dead_code)]

use selene_core::{
    CancellationChecker, DbString, GraphId, HnswIndexConfig, IvfIndexConfig, LabelSet, PropertyMap,
    Value, VectorMetric, VectorValue, db_string,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SeleneGraph, SharedGraph, VectorIndexConfig, VectorIndexKind,
    VectorIndexMemoryUsage, VectorNodeSearchHit,
};

const DISTANCE_TIE_EPSILON: f64 = 1e-9;
const HNSW_SEARCH_WIDTHS: &[usize] = &[10, 32, 64];
const IVF_SEARCH_WIDTHS: &[usize] = &[1, 2, 4, 8, 16, 32, 64];

pub(crate) const ANN_RECALL_PROFILES: &[AnnRecallProfile] = &[
    AnnRecallProfile::LineSquaredEuclidean,
    AnnRecallProfile::ClusteredCosine,
    AnnRecallProfile::NegativeInnerProduct,
];

#[derive(Clone, Copy, Debug)]
pub(crate) enum AnnRecallProfile {
    LineSquaredEuclidean,
    ClusteredCosine,
    NegativeInnerProduct,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AnnRecallVariant {
    pub(crate) name_suffix: &'static str,
    index: AnnIndexKind,
}

#[derive(Clone, Copy, Debug)]
enum AnnIndexKind {
    Hnsw {
        hnsw_config: Option<HnswIndexConfig>,
    },
    Ivf {
        ivf_config: Option<IvfIndexConfig>,
    },
}

static DEFAULT_ANN_RECALL_VARIANTS: [AnnRecallVariant; 2] = [
    AnnRecallVariant {
        name_suffix: "hnsw",
        index: AnnIndexKind::Hnsw { hnsw_config: None },
    },
    AnnRecallVariant {
        name_suffix: "ivf",
        index: AnnIndexKind::Ivf { ivf_config: None },
    },
];

static CLUSTERED_COSINE_ANN_RECALL_VARIANTS: [AnnRecallVariant; 3] = [
    AnnRecallVariant {
        name_suffix: "hnsw",
        index: AnnIndexKind::Hnsw { hnsw_config: None },
    },
    AnnRecallVariant {
        name_suffix: "hnsw_m24ef64",
        index: AnnIndexKind::Hnsw {
            hnsw_config: Some(HnswIndexConfig::new(24, 64)),
        },
    },
    AnnRecallVariant {
        name_suffix: "ivf",
        index: AnnIndexKind::Ivf { ivf_config: None },
    },
];

impl AnnRecallVariant {
    pub(crate) const fn ivf(name_suffix: &'static str, ivf_config: Option<IvfIndexConfig>) -> Self {
        Self {
            name_suffix,
            index: AnnIndexKind::Ivf { ivf_config },
        }
    }

    pub(crate) const fn is_ivf(self) -> bool {
        match self.index {
            AnnIndexKind::Hnsw { .. } => false,
            AnnIndexKind::Ivf { .. } => true,
        }
    }

    pub(crate) const fn search_widths(self) -> &'static [usize] {
        match self.index {
            AnnIndexKind::Hnsw { .. } => HNSW_SEARCH_WIDTHS,
            AnnIndexKind::Ivf { .. } => IVF_SEARCH_WIDTHS,
        }
    }

    const fn hnsw_config(self) -> Option<HnswIndexConfig> {
        match self.index {
            AnnIndexKind::Hnsw { hnsw_config } => hnsw_config,
            AnnIndexKind::Ivf { .. } => None,
        }
    }

    const fn ivf_config(self) -> Option<IvfIndexConfig> {
        match self.index {
            AnnIndexKind::Hnsw { .. } => None,
            AnnIndexKind::Ivf { ivf_config } => ivf_config,
        }
    }
}

impl AnnRecallProfile {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::LineSquaredEuclidean => "line_l2",
            Self::ClusteredCosine => "cluster_cos",
            Self::NegativeInnerProduct => "mips",
        }
    }

    pub(crate) const fn dimension(self) -> usize {
        match self {
            Self::LineSquaredEuclidean | Self::ClusteredCosine => 128,
            Self::NegativeInnerProduct => 64,
        }
    }

    pub(crate) fn variants(self) -> &'static [AnnRecallVariant] {
        match self {
            Self::ClusteredCosine => &CLUSTERED_COSINE_ANN_RECALL_VARIANTS,
            Self::LineSquaredEuclidean | Self::NegativeInnerProduct => &DEFAULT_ANN_RECALL_VARIANTS,
        }
    }

    const fn metric(self) -> VectorMetric {
        match self {
            Self::LineSquaredEuclidean => VectorMetric::SquaredEuclidean,
            Self::ClusteredCosine => VectorMetric::Cosine,
            Self::NegativeInnerProduct => VectorMetric::NegativeInnerProduct,
        }
    }

    const fn index_kind(self, variant: AnnRecallVariant) -> VectorIndexKind {
        match (self.metric(), variant.index) {
            (VectorMetric::SquaredEuclidean, AnnIndexKind::Hnsw { .. }) => {
                VectorIndexKind::HnswSquaredEuclidean
            }
            (VectorMetric::Cosine, AnnIndexKind::Hnsw { .. }) => VectorIndexKind::HnswCosine,
            (VectorMetric::NegativeInnerProduct, AnnIndexKind::Hnsw { .. }) => {
                VectorIndexKind::HnswNegativeInnerProduct
            }
            (VectorMetric::SquaredEuclidean, AnnIndexKind::Ivf { .. }) => {
                VectorIndexKind::IvfSquaredEuclidean
            }
            (VectorMetric::Cosine, AnnIndexKind::Ivf { .. }) => VectorIndexKind::IvfCosine,
            (VectorMetric::NegativeInnerProduct, AnnIndexKind::Ivf { .. }) => {
                VectorIndexKind::IvfNegativeInnerProduct
            }
        }
    }

    const fn graph_id_offset(self) -> u64 {
        match self {
            Self::LineSquaredEuclidean => 0,
            Self::ClusteredCosine => 1_000_000,
            Self::NegativeInnerProduct => 2_000_000,
        }
    }

    fn corpus_value(self, seed: usize, scale: usize) -> VectorValue {
        match self {
            Self::LineSquaredEuclidean => recall_corpus_value(seed, self.dimension()),
            Self::ClusteredCosine => clustered_cosine_value(seed, scale, self.dimension(), 0.0),
            Self::NegativeInnerProduct => mips_corpus_value(seed, scale, self.dimension()),
        }
    }

    fn query_value(self, query_idx: usize, scale: usize, query_count: usize) -> VectorValue {
        match self {
            Self::LineSquaredEuclidean => {
                recall_query_value(query_idx, scale, query_count, self.dimension())
            }
            Self::ClusteredCosine => {
                let cluster_count = recall_cluster_count(scale);
                let cluster = query_idx % cluster_count;
                let seed = cluster + (scale / cluster_count / 2) * cluster_count;
                clustered_cosine_value(
                    seed.min(scale.saturating_sub(1)),
                    scale,
                    self.dimension(),
                    0.0003,
                )
            }
            Self::NegativeInnerProduct => mips_query_value(query_idx, self.dimension()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AnnRecallFixture {
    profile: AnnRecallProfile,
    variant_name_suffix: &'static str,
    dimension: usize,
    scale: usize,
    graph: SeleneGraph,
    label: DbString,
    embedding_key: DbString,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<VectorNodeSearchHit>>,
    k: usize,
}

impl AnnRecallFixture {
    pub(crate) fn build(
        profile: AnnRecallProfile,
        variant: AnnRecallVariant,
        scale: usize,
        query_count: usize,
        k: usize,
    ) -> Self {
        let scale = scale.max(1);
        let dimension = profile.dimension();
        let label = db_string("AnnRecallDoc").expect("bench label is valid");
        let embedding_key = db_string("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(
            10_000 + scale as u64 + profile.graph_id_offset(),
        ));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let vector = Value::Vector(profile.corpus_value(idx, scale));
                let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                    .expect("bench vector properties are valid");
                mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
            }
            let dimension = u32::try_from(dimension).expect("bench dimension fits u32");
            mutator
                .create_vector_index_named_with_configs(
                    label.clone(),
                    embedding_key.clone(),
                    profile.index_kind(variant),
                    dimension,
                    None,
                    VectorIndexConfig::new(variant.hnsw_config(), variant.ivf_config()),
                )
                .expect("bench ANN vector index build succeeds");
            txn.commit()
                .expect("bench ANN recall fixture commit succeeds");
        }
        let graph = shared.read().as_ref().clone();
        let queries: Vec<_> = (0..query_count)
            .map(|idx| profile.query_value(idx, scale, query_count))
            .collect();
        let exact = queries
            .iter()
            .map(|query| {
                graph
                    .exact_vector_search_nodes(&label, &embedding_key, query, profile.metric(), k)
                    .expect("bench exact vector search succeeds")
            })
            .collect();
        Self {
            profile,
            variant_name_suffix: variant.name_suffix,
            dimension,
            scale,
            graph,
            label,
            embedding_key,
            queries,
            exact,
            k,
        }
    }

    pub(crate) const fn profile(&self) -> AnnRecallProfile {
        self.profile
    }

    pub(crate) const fn variant_name_suffix(&self) -> &'static str {
        self.variant_name_suffix
    }

    pub(crate) const fn dimension(&self) -> usize {
        self.dimension
    }

    pub(crate) const fn scale(&self) -> usize {
        self.scale
    }

    pub(crate) const fn query_count(&self) -> usize {
        self.queries.len()
    }

    pub(crate) fn mean_recall(&self, ef_search: usize) -> f64 {
        let expected = self.exact.iter().map(Vec::len).sum::<usize>();
        if expected == 0 {
            return 1.0;
        }
        self.total_overlap(ef_search) as f64 / expected as f64
    }

    pub(crate) fn mean_distance_quality(&self, ef_search: usize) -> f64 {
        let expected = self.exact.iter().map(Vec::len).sum::<usize>();
        if expected == 0 {
            return 1.0;
        }
        self.total_distance_quality(ef_search) as f64 / expected as f64
    }

    pub(crate) fn total_overlap(&self, ef_search: usize) -> usize {
        let approximate = self.approximate_batch(ef_search);
        self.exact
            .iter()
            .zip(&approximate)
            .map(|(exact, approximate)| overlap_count(exact, approximate))
            .sum()
    }

    pub(crate) fn memory_usage(&self) -> VectorIndexMemoryUsage {
        self.graph
            .vector_index_for(&self.label, &self.embedding_key)
            .expect("ANN recall fixture has a vector index")
            .memory_usage()
    }

    fn total_distance_quality(&self, ef_search: usize) -> usize {
        let approximate = self.approximate_batch(ef_search);
        self.exact
            .iter()
            .zip(&approximate)
            .map(|(exact, approximate)| distance_quality_count(exact, approximate))
            .sum()
    }

    fn approximate_batch(&self, ef_search: usize) -> Vec<Vec<VectorNodeSearchHit>> {
        self.graph
            .approximate_vector_search_nodes_batch_checked(
                &self.label,
                &self.embedding_key,
                &self.queries,
                ApproximateVectorSearchOptions::new(self.profile.metric(), self.k, ef_search),
                CancellationChecker::disabled(),
            )
            .expect("bench approximate vector batch search succeeds")
    }
}

fn recall_query_value(
    query_idx: usize,
    scale: usize,
    query_count: usize,
    dimension: usize,
) -> VectorValue {
    let seed = query_idx
        .saturating_mul(scale.max(1))
        .checked_div(query_count.max(1))
        .unwrap_or(0)
        .min(scale.saturating_sub(1));
    let mut components = recall_vector_components(seed, dimension);
    if let Some(first) = components.first_mut() {
        *first += 0.37;
    }
    VectorValue::new(components).expect("bench recall query is valid")
}

fn recall_corpus_value(seed: usize, dimension: usize) -> VectorValue {
    VectorValue::new(recall_vector_components(seed, dimension))
        .expect("bench recall corpus vector is valid")
}

fn clustered_cosine_value(
    seed: usize,
    scale: usize,
    dimension: usize,
    query_shift: f32,
) -> VectorValue {
    let cluster_count = recall_cluster_count(scale);
    let cluster = seed % cluster_count;
    let ordinal = seed / cluster_count;
    let center = cluster % dimension;
    let second = cluster.wrapping_mul(5).wrapping_add(3) % dimension;
    let spread = ordinal as f32 - (scale / cluster_count / 2) as f32;
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let base = (((cluster + 3) * (dim + 11)) % 17) as f32 / 200.0;
            let primary = if dim == center { 1.0 } else { 0.0 };
            let secondary = if dim == second { 0.25 } else { 0.0 };
            base + primary + secondary + spread * 0.0002 + query_shift
        })
        .collect();
    VectorValue::new(components).expect("bench clustered cosine vector is valid")
}

fn recall_cluster_count(scale: usize) -> usize {
    scale.clamp(1, 16)
}

fn mips_corpus_value(seed: usize, scale: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let trend = seed as f32 / scale.max(1) as f32;
            let local = ((seed * (dim + 13) + dim * 29) % 101) as f32 / 5_000.0;
            trend * (1.0 + dim as f32 / dimension as f32) + local + 0.01
        })
        .collect();
    VectorValue::new(components).expect("bench MIPS corpus vector is valid")
}

fn mips_query_value(query_idx: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let weight = 1.0 + dim as f32 / dimension as f32;
            let tilt = ((query_idx + dim * 7) % 23) as f32 / 1_000.0;
            weight + tilt
        })
        .collect();
    VectorValue::new(components).expect("bench MIPS query vector is valid")
}

fn recall_vector_components(seed: usize, dimension: usize) -> Vec<f32> {
    (0..dimension)
        .map(|dim| {
            if dim == 0 {
                seed as f32
            } else {
                let raw = seed
                    .wrapping_mul(dim.wrapping_mul(37).wrapping_add(11))
                    .wrapping_add(dim.wrapping_mul(31))
                    % 997;
                raw as f32 / 10_000.0
            }
        })
        .collect()
}

fn overlap_count(exact: &[VectorNodeSearchHit], approximate: &[VectorNodeSearchHit]) -> usize {
    exact
        .iter()
        .filter(|expected| {
            approximate
                .iter()
                .any(|hit| hit.node_id == expected.node_id)
        })
        .count()
}

fn distance_quality_count(
    exact: &[VectorNodeSearchHit],
    approximate: &[VectorNodeSearchHit],
) -> usize {
    let Some(threshold) = exact.last().map(|hit| hit.distance + DISTANCE_TIE_EPSILON) else {
        return 0;
    };
    approximate
        .iter()
        .take(exact.len())
        .filter(|hit| hit.distance <= threshold)
        .count()
}
