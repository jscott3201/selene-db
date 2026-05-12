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
use crate::snapshot::vecs::{VecsBodyV1, decode_vecs, encode_vecs};
use crate::{HnswConfig, HnswGraph, HnswParams, VectorError, hnsw, snapshot};

pub(crate) const PROVIDER_NAME: &str = "selene-vector";

/// Stateful vector index provider registered under the `VECT` provider tag.
///
/// This type validates configuration, declares the provider's snapshot
/// footprint, publishes immutable graph snapshots through ArcSwap, and replays
/// BRIEF-59 vector upsert events. BRIEF-60 adds HNSW search over published
/// snapshots. BRIEF-61 adds deterministic GRPH/VECS section codecs, and
/// BRIEF-62 adds the VECB bulk-insert event path. Later M8 briefs fill in
/// quantization; procedure registration moves to future selene-vector-pack
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
            snapshot::QUNT => {
                if bytes.is_empty() {
                    Ok(())
                } else {
                    Err(ProviderError::InvalidPayload {
                        reason: "QUNT section body is deferred to BRIEF-63; only empty payloads accepted".into(),
                    })
                }
            }
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
                let bytes = encode_grph(&captured, &self.config).map_err(section_encode_err)?;
                *self.staging.lock() = SectionStaging::Writing { captured };
                Ok(bytes)
            }
            snapshot::VECS => {
                let captured = match &*self.staging.lock() {
                    SectionStaging::Writing { captured } => Arc::clone(captured),
                    SectionStaging::Idle | SectionStaging::Reading { .. } => {
                        return Err(ProviderError::InvalidPayload {
                            reason: "VECS section write before GRPH".into(),
                        });
                    }
                };
                let bytes = encode_vecs(&captured).map_err(section_encode_err)?;
                // Codex review fix (Cluster A): clear staging after the
                // captured Arc is consumed so the snapshot generation does
                // not pin in memory across later upserts, and so a stray
                // VECS retry without a fresh GRPH cannot reuse stale state.
                // BRIEF-63 will revisit this lifetime when QUNT joins the
                // captured-Arc chain.
                *self.staging.lock() = SectionStaging::Idle;
                Ok(bytes)
            }
            snapshot::QUNT => Ok(Vec::new()),
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
