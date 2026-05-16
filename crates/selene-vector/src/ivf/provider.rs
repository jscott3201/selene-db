use std::collections::HashSet;
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use roaring::RoaringBitmap;
use selene_core::{Change, NodeId};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SubTag};

use super::coarse::CoarseQuantizer;
use super::mutate::{
    IvfEventKind, apply_bulk_delete, apply_bulk_insert, apply_payload, decode_ivf_event,
};
use super::train::{append_encoded, train};
use super::validate::{
    deferred_rows_to_raw, inconsistent, validate_posting_lists, validate_trained_codebook,
};
use super::{IVF_PROVIDER_NAME, IvfIndex, IvfStats, TrainedIvf, search};
use crate::payload::split_named_payload;
use crate::quantize::PqCodebook;
use crate::snapshot::cqnt::{CqntBodyV1, decode_cqnt, encode_cqnt};
use crate::snapshot::ipqb::{IpqbBodyV1, decode_ipqb, encode_ipqb};
use crate::snapshot::post::{PostBodyV1, decode_post, encode_post};
use crate::{IvfConfig, VectorError, snapshot};

/// Stateful vector index provider registered under the `IVFP` provider tag.
pub struct IvfProvider {
    config: IvfConfig,
    state: ArcSwap<IvfIndex>,
    staging: Mutex<SectionStaging>,
}

enum SectionStaging {
    Idle,
    Reading {
        cqnt: Option<CqntBodyV1>,
        ipqb: Option<IpqbBodyV1>,
    },
    Writing {
        captured: Arc<IvfIndex>,
        trained: Option<Arc<TrainedIvf>>,
        publish_after_post: bool,
        ipqb_emitted: bool,
    },
}

impl IvfProvider {
    /// Construct a provider from a validated config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when `config.validate()` fails.
    pub fn new(config: IvfConfig) -> Result<Self, VectorError> {
        config.validate()?;
        let dim = u16::try_from(config.dim).map_err(|_| VectorError::IvfDimensionMismatch {
            expected: u16::MAX as usize,
            observed: config.dim,
        })?;
        Ok(Self {
            config,
            state: ArcSwap::from_pointee(IvfIndex::empty(dim)),
            staging: Mutex::new(SectionStaging::Idle),
        })
    }

    /// Return this provider's immutable configuration.
    #[must_use]
    pub const fn config(&self) -> &IvfConfig {
        &self.config
    }

    /// Load the currently published immutable IVF snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<IvfIndex> {
        self.state.load_full()
    }

    /// Search the currently published IVF-PQ index.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::DimensionsLocked`] for query dimension
    /// mismatches, [`VectorError::NonFiniteQueryComponent`] for NaN/inf query
    /// components, or [`VectorError::IvfInvalidNProbe`] for bad probe counts.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        n_probe: Option<u32>,
        filter: Option<&RoaringBitmap>,
    ) -> Result<Vec<(NodeId, f32)>, VectorError> {
        let snapshot = self.state.load_full();
        search::search(&snapshot, query, k, n_probe, &self.config, filter)
    }

    /// Return statistics for the currently loaded IVF-PQ state.
    pub fn ivf_stats(&self) -> Result<Option<IvfStats>, VectorError> {
        let snapshot = self.state.load_full();
        if snapshot.is_empty() {
            return Ok(None);
        }
        if snapshot.is_trained() {
            return Ok(Some(stats_for(&snapshot, &self.config)));
        }
        if snapshot.len() < self.config.training_min_vectors {
            return Ok(Some(IvfStats::Deferred {
                observed_vectors: snapshot.len(),
                required: self.config.training_min_vectors,
            }));
        }
        Ok(None)
    }
}

impl IndexProvider for IvfProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        snapshot::IVFP
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        if !snapshot::is_declared_ivf(sub_tag) {
            return Err(unknown_sub_tag(sub_tag));
        }
        match sub_tag {
            snapshot::CQNT => self.read_cqnt(bytes),
            snapshot::IPQB => self.read_ipqb(bytes),
            snapshot::POST => self.read_post(bytes),
            _ => Err(unknown_sub_tag(sub_tag)),
        }
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        if !snapshot::is_declared_ivf(sub_tag) {
            return Err(unknown_sub_tag(sub_tag));
        }
        match sub_tag {
            snapshot::CQNT => self.write_cqnt(),
            snapshot::IPQB => self.write_ipqb(),
            snapshot::POST => self.write_post(),
            _ => Err(unknown_sub_tag(sub_tag)),
        }
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        if matches!(&*self.staging.lock(), SectionStaging::Reading { .. }) {
            return Err(ProviderError::InvalidPayload {
                reason: "incomplete IVF snapshot recovery in progress".into(),
            });
        }
        let (provider, payload) = match change {
            Change::IndexExtensionEvent { provider, payload } => (provider, payload),
            // No-op: the IVF provider only replays extension-owned WAL events.
            Change::NodeCreated { .. }
            | Change::NodeUpdated { .. }
            | Change::NodeDeleted { .. }
            | Change::EdgeCreated { .. }
            | Change::EdgeUpdated { .. }
            | Change::EdgeDeleted { .. }
            | Change::SchemaChanged { .. } => return Ok(()),
        };
        if provider.as_str() != IVF_PROVIDER_NAME {
            return Ok(());
        }
        let named =
            split_named_payload(payload.as_ref()).map_err(|err| ProviderError::InvalidPayload {
                reason: format!("selene-vector-ivf payload decode: prefix {err:?}: {err}"),
            })?;
        if named.index_name != "default" {
            return Err(ProviderError::Inconsistent {
                reason: format!(
                    "selene-vector-ivf singleton provider cannot apply vector index '{}'",
                    named.index_name
                ),
            });
        }
        let event = decode_ivf_event(&named.body).map_err(|err| ProviderError::InvalidPayload {
            reason: format!("selene-vector-ivf payload decode: {err:?}: {err}"),
        })?;
        let prev = self.state.load_full();
        let mut next = (*prev).clone();
        match event {
            IvfEventKind::Upsert(payload) => {
                apply_payload(&mut next, &payload, &self.config).map_err(|err| {
                    ProviderError::InvalidPayload {
                        reason: format!("selene-vector-ivf apply_upsert: {err:?}: {err}"),
                    }
                })?;
            }
            IvfEventKind::BulkInsert(payload) => {
                apply_bulk_insert(&mut next, &payload, &self.config).map_err(|err| {
                    ProviderError::InvalidPayload {
                        reason: format!("selene-vector-ivf apply_bulk_insert: {err:?}: {err}"),
                    }
                })?;
            }
            IvfEventKind::BulkDelete(payload) => {
                apply_bulk_delete(&mut next, &payload).map_err(|err| {
                    ProviderError::InvalidPayload {
                        reason: format!("selene-vector-ivf apply_bulk_delete: {err:?}: {err}"),
                    }
                })?;
            }
        }
        self.state.store(Arc::new(next));
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS_IVF
    }
}

impl IvfProvider {
    fn write_cqnt(&self) -> Result<Vec<u8>, ProviderError> {
        // Every error path inside this CQNT→IPQB→POST staging dance must
        // reset staging to Idle. Without the reset a mid-sequence
        // encode/publish failure leaves staging in `Writing`, and the next
        // snapshot attempt's IPQB/POST calls would silently proceed against
        // stale `captured` instead of failing fast on "section write before
        // CQNT".
        let captured = self.state.load_full();
        let trained = if captured.is_trained() || captured.len() < self.config.training_min_vectors
        {
            None
        } else {
            Some(Arc::new(
                train(&captured.unassigned_buffer, &self.config).map_err(|err| {
                    *self.staging.lock() = SectionStaging::Idle;
                    section_encode_err(err)
                })?,
            ))
        };
        let body = if let Some(trained) = &trained {
            CqntBodyV1::Trained {
                k_coarse: trained.coarse.k_coarse,
                dim: trained.coarse.dim,
                metric: snapshot::metric_to_wire(self.config.metric),
                centroids: trained.coarse.centroids.clone(),
            }
        } else if let Some(coarse) = captured.coarse_quantizer.as_deref() {
            CqntBodyV1::Trained {
                k_coarse: coarse.k_coarse,
                dim: coarse.dim,
                metric: snapshot::metric_to_wire(self.config.metric),
                centroids: coarse.centroids.clone(),
            }
        } else {
            CqntBodyV1::Empty
        };
        let bytes = encode_cqnt(&body).map_err(|err| {
            *self.staging.lock() = SectionStaging::Idle;
            section_encode_err(err)
        })?;
        *self.staging.lock() = SectionStaging::Writing {
            captured: Arc::clone(&captured),
            publish_after_post: trained.is_some(),
            trained,
            ipqb_emitted: false,
        };
        Ok(bytes)
    }

    fn write_ipqb(&self) -> Result<Vec<u8>, ProviderError> {
        let (captured, trained, publish_after_post) = match &*self.staging.lock() {
            SectionStaging::Writing {
                captured,
                trained,
                publish_after_post,
                ..
            } => (Arc::clone(captured), trained.clone(), *publish_after_post),
            SectionStaging::Idle | SectionStaging::Reading { .. } => {
                return Err(ProviderError::InvalidPayload {
                    reason: "IPQB section write before CQNT".into(),
                });
            }
        };
        let body = if let Some(trained) = &trained {
            IpqbBodyV1::Trained {
                codebook: (*trained.codebook).clone(),
            }
        } else if let Some(codebook) = captured.residual_codebook.as_deref() {
            IpqbBodyV1::Trained {
                codebook: codebook.clone(),
            }
        } else {
            IpqbBodyV1::Empty
        };
        let bytes = encode_ipqb(&body).map_err(|err| {
            *self.staging.lock() = SectionStaging::Idle;
            section_encode_err(err)
        })?;
        *self.staging.lock() = SectionStaging::Writing {
            captured,
            trained,
            publish_after_post,
            ipqb_emitted: true,
        };
        Ok(bytes)
    }

    fn write_post(&self) -> Result<Vec<u8>, ProviderError> {
        let (captured, trained, publish_after_post) = {
            let mut staging = self.staging.lock();
            match &*staging {
                SectionStaging::Writing {
                    captured,
                    trained,
                    publish_after_post,
                    ipqb_emitted: true,
                } => (Arc::clone(captured), trained.clone(), *publish_after_post),
                SectionStaging::Writing {
                    ipqb_emitted: false,
                    ..
                } => {
                    *staging = SectionStaging::Idle;
                    return Err(ProviderError::InvalidPayload {
                        reason: "POST section write before CQNT+IPQB".into(),
                    });
                }
                SectionStaging::Idle | SectionStaging::Reading { .. } => {
                    return Err(ProviderError::InvalidPayload {
                        reason: "POST section write before CQNT+IPQB".into(),
                    });
                }
            }
        };
        // Every fallible step below maps Err through `reset_then` so a
        // mid-flight POST failure cannot leave staging in `Writing`.
        let reset_then = |err: ProviderError| -> ProviderError {
            *self.staging.lock() = SectionStaging::Idle;
            err
        };
        let body = if let Some(trained) = &trained {
            PostBodyV1::Trained {
                node_count: u32::try_from(captured.len()).map_err(|_| {
                    reset_then(section_encode_err(snapshot::encode_failed(
                        snapshot::POST,
                        "IVF node_count exceeds POST range",
                    )))
                })?,
                posting_lists: trained.posting_lists.clone(),
            }
        } else if captured.is_trained() {
            PostBodyV1::Trained {
                node_count: u32::try_from(captured.len()).map_err(|_| {
                    reset_then(section_encode_err(snapshot::encode_failed(
                        snapshot::POST,
                        "IVF node_count exceeds POST range",
                    )))
                })?,
                posting_lists: captured.posting_lists.clone(),
            }
        } else if captured.unassigned_buffer.is_empty() {
            PostBodyV1::Empty
        } else {
            PostBodyV1::Deferred {
                unassigned_rows: captured
                    .unassigned_buffer
                    .iter()
                    .map(|row| (row.node_id, row.vector.to_vec()))
                    .collect(),
            }
        };
        let bytes = encode_post(&body).map_err(|err| reset_then(section_encode_err(err)))?;
        if publish_after_post && let Some(trained) = trained {
            self.publish_trained_merge(&captured, &trained)
                .map_err(reset_then)?;
        }
        *self.staging.lock() = SectionStaging::Idle;
        Ok(bytes)
    }

    fn read_cqnt(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        *self.staging.lock() = SectionStaging::Idle;
        let body = decode_cqnt(bytes).map_err(section_decode_err)?;
        *self.staging.lock() = SectionStaging::Reading {
            cqnt: Some(body),
            ipqb: None,
        };
        Ok(())
    }

    fn read_ipqb(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let body = match decode_ipqb(bytes) {
            Ok(body) => body,
            Err(err) => {
                *self.staging.lock() = SectionStaging::Idle;
                return Err(section_decode_err(err));
            }
        };
        let mut staging = self.staging.lock();
        match &mut *staging {
            SectionStaging::Reading {
                cqnt: Some(_),
                ipqb,
            } => {
                *ipqb = Some(body);
                Ok(())
            }
            SectionStaging::Idle
            | SectionStaging::Writing { .. }
            | SectionStaging::Reading { cqnt: None, .. } => {
                *staging = SectionStaging::Idle;
                Err(ProviderError::InvalidPayload {
                    reason: "IPQB section read before CQNT".into(),
                })
            }
        }
    }

    fn read_post(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let post = match decode_post(bytes) {
            Ok(body) => body,
            Err(err) => {
                *self.staging.lock() = SectionStaging::Idle;
                return Err(section_decode_err(err));
            }
        };
        let (cqnt, ipqb) = {
            let mut staging = self.staging.lock();
            let previous = std::mem::replace(&mut *staging, SectionStaging::Idle);
            match previous {
                SectionStaging::Reading {
                    cqnt: Some(cqnt),
                    ipqb: Some(ipqb),
                } => (cqnt, ipqb),
                _ => {
                    return Err(ProviderError::InvalidPayload {
                        reason: "POST section read before CQNT+IPQB".into(),
                    });
                }
            }
        };
        let index = assemble_index(cqnt, ipqb, post, &self.config).map_err(section_decode_err)?;
        self.state.store(Arc::new(index));
        Ok(())
    }

    fn publish_trained_merge(
        &self,
        captured: &IvfIndex,
        trained: &TrainedIvf,
    ) -> Result<(), ProviderError> {
        let captured_ids = captured
            .unassigned_buffer
            .iter()
            .map(|row| row.node_id)
            .collect::<HashSet<_>>();
        let current = self.state.load_full();
        let mut posting_lists = trained.posting_lists.clone();
        let mut vector_count = captured.len();
        for row in &current.unassigned_buffer {
            if captured_ids.contains(&row.node_id) {
                continue;
            }
            append_encoded(
                &mut posting_lists,
                &trained.coarse,
                &trained.codebook,
                row.node_id,
                &row.vector,
                &self.config,
            )
            .map_err(section_encode_err)?;
            vector_count += 1;
        }
        let merged = IvfIndex::trained(
            u16::try_from(self.config.dim).map_err(|_| {
                section_encode_err(snapshot::encode_failed(
                    snapshot::POST,
                    "provider dim exceeds u16",
                ))
            })?,
            TrainedIvf {
                coarse: Arc::clone(&trained.coarse),
                codebook: Arc::clone(&trained.codebook),
                posting_lists,
            },
            vector_count,
        );
        self.state.store(Arc::new(merged));
        Ok(())
    }
}

fn assemble_index(
    cqnt: CqntBodyV1,
    ipqb: IpqbBodyV1,
    post: PostBodyV1,
    config: &IvfConfig,
) -> Result<IvfIndex, VectorError> {
    let dim = u16::try_from(config.dim)
        .map_err(|_| snapshot::decode_failed(snapshot::CQNT, "provider dim exceeds CQNT range"))?;
    match (cqnt, ipqb, post) {
        (CqntBodyV1::Empty, IpqbBodyV1::Empty, PostBodyV1::Empty) => Ok(IvfIndex::empty(dim)),
        (CqntBodyV1::Empty, IpqbBodyV1::Empty, PostBodyV1::Deferred { unassigned_rows }) => {
            let rows = deferred_rows_to_raw(unassigned_rows, config)?;
            Ok(IvfIndex::deferred(dim, rows))
        }
        (
            CqntBodyV1::Trained {
                k_coarse,
                dim: body_dim,
                metric,
                centroids,
            },
            IpqbBodyV1::Trained { codebook },
            PostBodyV1::Trained {
                node_count,
                posting_lists,
            },
        ) => {
            if k_coarse != config.k_coarse
                || body_dim != dim
                || metric != snapshot::metric_to_wire(config.metric)
            {
                return Err(inconsistent("CQNT disagrees with provider config"));
            }
            validate_trained_codebook(&codebook, config)?;
            validate_posting_lists(&posting_lists, node_count, config, &codebook)?;
            let coarse = Arc::new(CoarseQuantizer {
                centroids,
                k_coarse,
                dim: body_dim,
            });
            Ok(IvfIndex::trained(
                dim,
                TrainedIvf {
                    coarse,
                    codebook: Arc::new(codebook),
                    posting_lists,
                },
                node_count as usize,
            ))
        }
        _ => Err(inconsistent(
            "CQNT/IPQB/POST must be all empty, deferred, or all trained",
        )),
    }
}

fn stats_for(index: &IvfIndex, config: &IvfConfig) -> IvfStats {
    let posting_list_lengths = index
        .posting_lists
        .iter()
        .map(|list| u32::try_from(list.entries.len()).unwrap_or(u32::MAX))
        .collect::<Vec<_>>();
    let bytes_coarse_centroids = config
        .k_coarse
        .saturating_mul(config.dim as u32)
        .saturating_mul(std::mem::size_of::<f32>() as u32)
        as usize;
    let bytes_residual_codebook = index
        .residual_codebook
        .as_deref()
        .map_or(0, PqCodebook::bytes_codebook);
    let bytes_rotation = index
        .residual_codebook
        .as_deref()
        .map_or(0, PqCodebook::bytes_rotation);
    let bytes_posting_lists = index
        .posting_lists
        .iter()
        .flat_map(|list| &list.entries)
        .map(|entry| entry.codes.len())
        .sum();
    let bytes_reconstructed_norms = index
        .posting_lists
        .iter()
        .flat_map(|list| &list.entries)
        .filter(|entry| entry.reconstructed_norm.is_some())
        .count()
        .saturating_mul(std::mem::size_of::<f32>());
    let compressed = bytes_coarse_centroids
        .saturating_add(bytes_residual_codebook)
        .saturating_add(bytes_rotation)
        .saturating_add(bytes_posting_lists)
        .saturating_add(bytes_reconstructed_norms);
    let f32_bytes = index
        .len()
        .saturating_mul(config.dim)
        .saturating_mul(std::mem::size_of::<f32>());
    let polysemous = index
        .residual_codebook
        .as_deref()
        .is_some_and(|codebook| codebook.polysemous_trained);
    IvfStats::Trained {
        k_coarse: config.k_coarse,
        n_probe_default: config.n_probe,
        posting_list_lengths,
        unassigned_count: index.unassigned_buffer.len(),
        bytes_coarse_centroids,
        bytes_residual_codebook,
        bytes_rotation,
        bytes_posting_lists,
        bytes_reconstructed_norms,
        compression_ratio: if compressed == 0 {
            0.0
        } else {
            f32_bytes as f32 / compressed as f32
        },
        polysemous,
    }
}

fn section_decode_err(err: VectorError) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("selene-vector-ivf section decode: {err:?}: {err}"),
    }
}

fn section_encode_err(err: VectorError) -> ProviderError {
    ProviderError::SerializationFailed {
        reason: format!("selene-vector-ivf section encode: {err:?}: {err}"),
    }
}

fn unknown_sub_tag(sub_tag: SubTag) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("unknown selene-vector-ivf sub_tag {sub_tag}"),
    }
}
