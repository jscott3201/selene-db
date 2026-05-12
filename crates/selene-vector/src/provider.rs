//! HNSW index-provider skeleton for selene-vector.

use std::sync::Arc;

use arc_swap::ArcSwap;
use selene_core::Change;
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SubTag};

use crate::{HnswConfig, HnswGraph, VectorError, snapshot};

/// Stateful vector index provider registered under the `VECT` provider tag.
///
/// BRIEF-57 keeps this type deliberately small. It validates configuration,
/// declares the provider's snapshot footprint, and round-trips empty section
/// bodies. BRIEF-58 adds the immutable graph publication slot; later M8 briefs
/// fill it through WAL replay, snapshot codecs, and query behavior.
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

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        // Why: BRIEF-59 wires Change::IndexExtensionEvent replay. Reading the
        // snapshot here keeps the state field intentionally live while proving
        // this callback is non-mutating in BRIEF-58.
        let _snapshot = self.state.load();
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
