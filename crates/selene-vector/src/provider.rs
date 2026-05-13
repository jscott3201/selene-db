//! HNSW index provider for selene-vector.

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use roaring::RoaringBitmap;
use selene_core::{Change, NodeId};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SubTag};

use crate::builder::{apply_bulk_upsert, apply_upsert};
use crate::payload::{EventKind, decode_event};
use crate::snapshot::grph::{GrphHeaderV1, GrphNodeV1, decode_grph, encode_grph};
use crate::snapshot::qunt::{decode_qunt, encode_qunt, validate_qunt_for_graph};
use crate::snapshot::vecs::{VecsBodyV1, decode_vecs, encode_vecs};
use crate::{
    HnswConfig, HnswGraph, HnswParams, PqParams, QuantMethod, QuantizationStats, VectorError, hnsw,
    snapshot,
};

pub(crate) const PROVIDER_NAME: &str = "selene-vector";

/// Stateful vector index provider registered under the `VECT` provider tag.
///
/// This type validates configuration, declares the provider's snapshot
/// footprint, publishes immutable graph snapshots through ArcSwap, and replays
/// BRIEF-59 vector upsert events. BRIEF-60 adds HNSW search over published
/// snapshots. BRIEF-61 adds deterministic GRPH/VECS section codecs, BRIEF-62
/// adds the VECB bulk-insert event path, and BRIEF-63 adds the optional QUNT
/// SQ8 overlay. Procedure registration moves to future selene-vector-pack
/// work.
pub struct HnswProvider {
    config: HnswConfig,
    state: ArcSwap<HnswGraph>,
    staging: Mutex<SectionStaging>,
}

enum SectionStaging {
    Idle,
    Reading {
        grph: Option<(GrphHeaderV1, Vec<GrphNodeV1>)>,
        vecs: Option<VecsBodyV1>,
    },
    Writing {
        captured: Arc<HnswGraph>,
        expects_qunt: bool,
        vecs_emitted: bool,
    },
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
            staging: Mutex::new(SectionStaging::Idle),
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

    /// Return statistics for the currently loaded quantized store, if any.
    pub fn quantization_stats(&self) -> Result<Option<QuantizationStats>, VectorError> {
        let snapshot = self.state.load_full();
        if let Some(store) = snapshot.quantized() {
            return Ok(Some(store.stats()));
        }
        if self.config.quantization.enabled && self.config.quantization.method == QuantMethod::Pq {
            let params = PqParams::resolve(self.config.dim, self.config.quantization.pq);
            let observed = snapshot.len();
            if observed < params.train_min_vectors {
                return Err(VectorError::PqTrainingDeferred {
                    observed_vectors: observed,
                    required: params.train_min_vectors,
                });
            }
        }
        Ok(None)
    }

    /// Apply a single upsert payload through the same code path as
    /// [`Self::on_change`], but return the underlying [`VectorError`] instead
    /// of wrapping it into [`ProviderError`]. Test-harness only — exposes a
    /// typed view of `apply_upsert` so snapshot fixtures can render the real
    /// variant fields (Codex review fix, PR #72 P2-A).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn apply_upsert_for_test(
        &self,
        payload: &crate::payload::VectorUpsertPayloadV1,
    ) -> Result<(), VectorError> {
        let prev = self.state.load_full();
        let next = apply_upsert(&prev, payload, &self.config)?;
        self.state.store(Arc::new(next));
        Ok(())
    }
}

impl IndexProvider for HnswProvider {
    fn provider_tag(&self) -> ProviderTag {
        snapshot::VECT
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        if !snapshot::is_declared(sub_tag) {
            return Err(unknown_sub_tag(sub_tag));
        }
        match sub_tag {
            snapshot::GRPH => self.read_grph(bytes),
            snapshot::VECS => self.read_vecs(bytes),
            snapshot::QUNT => self.read_qunt(bytes),
            _ => Err(unknown_sub_tag(sub_tag)),
        }
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        if !snapshot::is_declared(sub_tag) {
            return Err(unknown_sub_tag(sub_tag));
        }
        match sub_tag {
            snapshot::GRPH => {
                // Codex review fix (Cluster B): capture into staging AFTER
                // encode succeeds. Encoding before staging means a failed
                // encode never leaves a stale `Writing { captured }` behind
                // that a subsequent VECS retry could reuse.
                let captured = self.state.load_full();
                let bytes = match encode_grph(&captured, &self.config) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        *self.staging.lock() = SectionStaging::Idle;
                        return Err(section_encode_err(err));
                    }
                };
                *self.staging.lock() = SectionStaging::Writing {
                    captured,
                    expects_qunt: self.config.quantization.enabled,
                    vecs_emitted: false,
                };
                Ok(bytes)
            }
            snapshot::VECS => {
                let (captured, expects_qunt) = match &*self.staging.lock() {
                    SectionStaging::Writing {
                        captured,
                        expects_qunt,
                        ..
                    } => (Arc::clone(captured), *expects_qunt),
                    SectionStaging::Idle | SectionStaging::Reading { .. } => {
                        return Err(ProviderError::InvalidPayload {
                            reason: "VECS section write before GRPH".into(),
                        });
                    }
                };
                let bytes = match encode_vecs(&captured) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        *self.staging.lock() = SectionStaging::Idle;
                        return Err(section_encode_err(err));
                    }
                };
                // Codex review fix (Cluster A): clear staging after the
                // captured Arc is consumed so the snapshot generation does
                // not pin in memory across later upserts, and so a stray
                // VECS retry without a fresh GRPH cannot reuse stale state.
                if expects_qunt {
                    *self.staging.lock() = SectionStaging::Writing {
                        captured,
                        expects_qunt,
                        vecs_emitted: true,
                    };
                } else {
                    *self.staging.lock() = SectionStaging::Idle;
                }
                Ok(bytes)
            }
            snapshot::QUNT => self.write_qunt(),
            _ => Err(unknown_sub_tag(sub_tag)),
        }
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        // Codex review fix (Cluster C): refuse mutation events while a GRPH
        // section has been staged without a matching VECS commit. Otherwise
        // an incomplete or truncated snapshot would silently route WAL
        // replay into the pre-recovery in-memory graph, discarding the
        // staged topology and producing data loss.
        if let SectionStaging::Reading {
            grph: Some(_),
            vecs: None,
        } = &*self.staging.lock()
        {
            return Err(ProviderError::InvalidPayload {
                reason: "incomplete provider snapshot: GRPH section staged without VECS".into(),
            });
        }
        let Change::IndexExtensionEvent { provider, payload } = change else {
            return Ok(());
        };
        if provider.as_str() != PROVIDER_NAME {
            return Ok(());
        }
        let event =
            decode_event(payload.as_ref()).map_err(|err| ProviderError::InvalidPayload {
                reason: format!("selene-vector payload decode: {err:?}: {err}"),
            })?;
        let prev = self.state.load_full();
        let next =
            match event {
                EventKind::Upsert(payload) => {
                    apply_upsert(&prev, &payload, &self.config).map_err(|err| {
                        ProviderError::InvalidPayload {
                            reason: format!("selene-vector apply_upsert: {err:?}: {err}"),
                        }
                    })?
                }
                EventKind::Bulk(payload) => apply_bulk_upsert(&prev, &payload, &self.config)
                    .map_err(|err| ProviderError::InvalidPayload {
                        reason: format!("selene-vector apply_bulk_upsert: {err:?}: {err}"),
                    })?,
            };
        self.state.store(Arc::new(next));
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS
    }
}

impl HnswProvider {
    fn read_grph(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        *self.staging.lock() = SectionStaging::Idle;
        let decoded = decode_grph(bytes).map_err(section_decode_err)?;
        snapshot::validate_config(&decoded.0, &self.config).map_err(section_decode_err)?;
        *self.staging.lock() = SectionStaging::Reading {
            grph: Some(decoded),
            vecs: None,
        };
        Ok(())
    }

    fn read_vecs(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let vecs = match decode_vecs(bytes) {
            Ok(vecs) => vecs,
            Err(err) => {
                *self.staging.lock() = SectionStaging::Idle;
                return Err(section_decode_err(err));
            }
        };
        let (header, nodes, vecs) = {
            let mut staging = self.staging.lock();
            match &mut *staging {
                SectionStaging::Reading {
                    grph: Some(_),
                    vecs: staged_vecs,
                } => {
                    *staged_vecs = Some(vecs);
                }
                SectionStaging::Idle
                | SectionStaging::Writing { .. }
                | SectionStaging::Reading { grph: None, .. } => {
                    *staging = SectionStaging::Idle;
                    return Err(ProviderError::InvalidPayload {
                        reason: "VECS section read before GRPH".into(),
                    });
                }
            }
            let previous = std::mem::replace(&mut *staging, SectionStaging::Idle);
            match previous {
                SectionStaging::Reading {
                    grph: Some(grph),
                    vecs: Some(vecs),
                } => (grph.0, grph.1, vecs),
                _ => unreachable!("read staging checked before replacement"),
            }
        };
        let graph = snapshot::assemble_graph(header, nodes, vecs, &self.config)
            .map_err(section_decode_err)?;
        self.state.store(Arc::new(graph));
        Ok(())
    }

    fn read_qunt(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        // Codex review fix (P1): preserve the BRIEF-61 incomplete-recovery
        // marker. Resetting staging on entry would let an empty QUNT clear a
        // staged-without-VECS Reading state, bypassing the on_change guard.
        // It would also let a non-empty QUNT validate against the previously
        // published graph (self.state.load_full()) rather than the staged
        // recovery target, potentially attaching the overlay to the wrong
        // graph when dimensions and counts coincide.
        match &*self.staging.lock() {
            SectionStaging::Reading {
                grph: Some(_),
                vecs: None,
            } => {
                return Err(ProviderError::InvalidPayload {
                    reason: "QUNT section read before VECS commit".into(),
                });
            }
            SectionStaging::Writing { .. } => {
                return Err(ProviderError::InvalidPayload {
                    reason: "QUNT section read during snapshot write".into(),
                });
            }
            SectionStaging::Idle
            | SectionStaging::Reading { grph: None, .. }
            | SectionStaging::Reading {
                grph: Some(_),
                vecs: Some(_),
            } => {}
        }

        if bytes.is_empty() {
            self.clear_quantized();
            return Ok(());
        }

        let body = decode_qunt(bytes).map_err(section_decode_err)?;
        let snapshot = self.state.load_full();
        validate_qunt_for_graph(&body, &snapshot, &self.config).map_err(section_decode_err)?;
        let store = Arc::new(body.store);
        // §M Q1: QUNT is a post-commit overlay, attached with RCU.
        self.state
            .rcu(|prev| prev.clone_with_quantized(Some(Arc::clone(&store))));
        Ok(())
    }

    fn write_qunt(&self) -> Result<Vec<u8>, ProviderError> {
        if !self.config.quantization.enabled {
            // §M Q2: empty QUNT emission still clears any captured Arc.
            *self.staging.lock() = SectionStaging::Idle;
            return Ok(Vec::new());
        }

        let captured = {
            let mut staging = self.staging.lock();
            match &*staging {
                SectionStaging::Writing {
                    captured,
                    vecs_emitted: true,
                    ..
                } => Arc::clone(captured),
                SectionStaging::Idle
                | SectionStaging::Reading { .. }
                | SectionStaging::Writing {
                    vecs_emitted: false,
                    ..
                } => {
                    *staging = SectionStaging::Idle;
                    return Err(ProviderError::InvalidPayload {
                        reason: "QUNT section write before GRPH+VECS".into(),
                    });
                }
            }
        };
        let (bytes, store) = match encode_qunt(&captured, &self.config) {
            Ok(encoded) => encoded,
            Err(err) => {
                *self.staging.lock() = SectionStaging::Idle;
                return Err(section_encode_err(err));
            }
        };
        if let Some(store) = store {
            let store = Arc::new(store);
            self.state
                .rcu(|prev| prev.clone_with_quantized(Some(Arc::clone(&store))));
        }
        *self.staging.lock() = SectionStaging::Idle;
        Ok(bytes)
    }

    fn clear_quantized(&self) {
        self.state.rcu(|prev| prev.clone_with_quantized(None));
    }
}

fn section_decode_err(err: VectorError) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("selene-vector section decode: {err:?}: {err}"),
    }
}

fn section_encode_err(err: VectorError) -> ProviderError {
    ProviderError::SerializationFailed {
        reason: format!("selene-vector section encode: {err:?}: {err}"),
    }
}

fn unknown_sub_tag(sub_tag: SubTag) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("unknown selene-vector sub_tag {sub_tag}"),
    }
}
