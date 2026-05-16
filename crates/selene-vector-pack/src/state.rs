//! Shared vector-pack state.

use selene_core::NodeId;
use xxhash_rust::xxh3::xxh3_64_with_seed;

/// Configuration shared by all vector-pack procedures.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VectorPackConfig {
    /// Optional deterministic seed for HNSW layer assignment.
    ///
    /// When `Some(seed)`, `vector.upsert` and `vector.bulk_upsert` derive
    /// each row's layer RNG from that seed plus the stable `NodeId`, making
    /// snapshot bytes reproducible across processes given identical input.
    /// `None` preserves the existing entropy-seeded behavior.
    pub deterministic_seed: Option<u64>,
}

/// Empty state handle reserved for future vector-pack adapters.
#[derive(Clone, Debug, Default)]
pub struct VectorPackState {
    config: VectorPackConfig,
}

impl VectorPackState {
    /// Construct state from explicit vector-pack configuration.
    #[must_use]
    pub const fn new(config: VectorPackConfig) -> Self {
        Self { config }
    }

    /// Return the active vector-pack configuration.
    #[must_use]
    pub const fn config(&self) -> &VectorPackConfig {
        &self.config
    }

    pub(crate) fn layer_rng_for_node(&self, node_id: NodeId) -> Option<fastrand::Rng> {
        self.config.deterministic_seed.map(|seed| {
            let row_seed = xxh3_64_with_seed(&node_id.get().to_le_bytes(), seed);
            fastrand::Rng::with_seed(row_seed)
        })
    }
}
