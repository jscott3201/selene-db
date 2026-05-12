//! HNSW index provider for selene-vector.

use std::sync::Arc;

use arc_swap::ArcSwap;
use roaring::RoaringBitmap;
use selene_core::{Change, NodeId};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SubTag};

use crate::builder::apply_upsert;
use crate::payload::VectorUpsertPayloadV1;
use crate::{HnswConfig, HnswGraph, HnswParams, VectorError, hnsw, snapshot};

pub(crate) const PROVIDER_NAME: &str = "selene-vector";

/// Stateful vector index provider registered under the `VECT` provider tag.
///
/// This type validates configuration, declares the provider's snapshot
/// footprint, publishes immutable graph snapshots through ArcSwap, and replays
/// BRIEF-59 vector upsert events. BRIEF-60 adds HNSW search over published
/// snapshots. Later M8 briefs fill in snapshot codecs, procedures, and
/// quantization.
pub struct HnswProvider {
    config: HnswConfig,
    state: ArcSwap<HnswGraph>,
}

impl HnswProvider {
    /// Construct a provider from a validated config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when `config.validate()` fails.
    pub fn new(config: HnswConfig) -> Result<Self, VectorError> {
        config.validate()?;
        let initial = HnswGraph::empty(config.dim as u16);
        Ok(Self {
            config,
            state: ArcSwap::from_pointee(initial),
        })
    }

    /// Return this provider's immutable configuration.
    #[must_use]
    pub const fn config(&self) -> &HnswConfig {
        &self.config
    }

    /// Load the currently published immutable HNSW graph snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<HnswGraph> {
        self.state.load_full()
    }

    /// Search the currently published HNSW snapshot for the top-`k` neighbors
    /// of `query`, optionally filtered by raw-NodeId bitmap membership.
    ///
    /// `ef_search` overrides the configured search width. Pass `None` to use
    /// the value from [`HnswConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::DimensionsLocked`] when `query.len()` disagrees
    /// with the provider's configured dimension, or
    /// [`VectorError::NonFiniteQueryComponent`] when the query contains NaN or
    /// infinity.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        ef_search: Option<usize>,
        filter: Option<&RoaringBitmap>,
    ) -> Result<Vec<(NodeId, f32)>, VectorError> {
        let snapshot = self.state.load_full();
        let params = HnswParams::from_config(&self.config);
        let ef = ef_search.unwrap_or(self.config.ef_search);
        hnsw::search::search(&snapshot, query, k, ef, &params, filter)
    }
}

impl IndexProvider for HnswProvider {
    fn provider_tag(&self) -> ProviderTag {
        snapshot::VECT
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        // Why: BRIEFs 61 and 63 replace this with real rkyv body decoders.
        // BRIEF-57 only proves the provider registration and empty-state
        // routing contract.
        if !snapshot::is_declared(sub_tag) {
            return Err(unknown_sub_tag(sub_tag));
        }
        if bytes.is_empty() {
            Ok(())
        } else {
            Err(ProviderError::InvalidPayload {
                reason: format!(
                    "selene-vector v0.1.0: section {sub_tag} body decoder not yet \
                     implemented (lands in BRIEF-61/63); only empty payloads accepted"
                ),
            })
        }
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        // Why: BRIEF-57 represents zero-state provider sections as empty
        // payloads; BRIEF-61 replaces this with deterministic section bytes.
        if snapshot::is_declared(sub_tag) {
            Ok(Vec::new())
        } else {
            Err(unknown_sub_tag(sub_tag))
        }
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        let Change::IndexExtensionEvent { provider, payload } = change else {
            return Ok(());
        };
        if provider.as_str() != PROVIDER_NAME {
            return Ok(());
        }
        let parsed = VectorUpsertPayloadV1::decode(payload.as_ref()).map_err(|err| {
            ProviderError::InvalidPayload {
                reason: format!("selene-vector payload decode: {err:?}: {err}"),
            }
        })?;
        let prev = self.state.load_full();
        let next = apply_upsert(&prev, &parsed, &self.config).map_err(|err| {
            ProviderError::InvalidPayload {
                reason: format!("selene-vector apply_upsert: {err:?}: {err}"),
            }
        })?;
        self.state.store(Arc::new(next));
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS
    }
}

fn unknown_sub_tag(sub_tag: SubTag) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("unknown selene-vector sub_tag {sub_tag}"),
    }
}
