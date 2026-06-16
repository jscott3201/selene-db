use super::*;

/// A monotonic per-flush watermark ("flush epoch") shared by [`FlushEpochDurable`]
/// (the writer/fsync side) and [`FlushEpochObserver`] (the publish/fan-out side),
/// so a test can pin the R1 barrier's interleaving: append happens at epoch E,
/// the group flush bumps the epoch E->E+1, and publish (fan-out) is observed at
/// the post-flush epoch. `publish_epoch > append_epoch` therefore proves the
/// commit was flushed strictly between its Stage-1 append and its Stage-3 publish
/// for every batched commit, and at the compact boundary in particular.
pub(super) type FlushEpoch = Arc<AtomicU64>;

/// Durable that latches a shared per-flush watermark and records, for every
/// `write_commit`, both the assigned sequence and the flush-epoch in effect when
/// it was appended (BEFORE its group flush). Paired with [`FlushEpochObserver`],
/// it proves a batch's commits were FLUSHED before they were published (and, at
/// the compact boundary, before the dense Arc stored).
pub(super) struct FlushEpochDurable {
    tag: ProviderTag,
    seq: AtomicU64,
    /// Shared monotonic flush watermark, bumped once per `flush`.
    flush_epoch: FlushEpoch,
    /// Count of `flush` calls (used by the zero-flush compact-at-head proof).
    pub(super) flushes: AtomicU64,
    /// Per assigned sequence, the flush-epoch observed at append time.
    append_epoch: Mutex<BTreeMap<u64, u64>>,
}

impl FlushEpochDurable {
    pub(super) fn new(tag: &[u8; 4], flush_epoch: FlushEpoch) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            flush_epoch,
            flushes: AtomicU64::new(0),
            append_epoch: Mutex::new(BTreeMap::new()),
        })
    }

    /// The flush-epoch in effect when the commit assigned `seq` was appended.
    pub(super) fn append_epoch_of(&self, seq: u64) -> u64 {
        *self
            .append_epoch
            .lock()
            .unwrap()
            .get(&seq)
            .expect("append epoch recorded for seq")
    }
}

impl DurableProvider for FlushEpochDurable {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }
    fn write_commit(
        &self,
        _principal: Option<&Arc<[u8]>>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        // Record the flush-epoch in effect at append time, BEFORE any group flush
        // for this run runs (Stage 1, fsync deferred).
        let epoch = self.flush_epoch.load(Ordering::SeqCst);
        self.append_epoch.lock().unwrap().insert(seq, epoch);
        Ok(seq)
    }
    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        // The R1 barrier: bump the shared watermark so any publish observed after
        // this flush reads a strictly-greater epoch than the appends it covers.
        self.flush_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.seq.load(Ordering::SeqCst)))
    }
}

/// Fan-out provider that records, per published `NodeCreated`, the flush-epoch in
/// effect at publish time (Stage 3). Paired with [`FlushEpochDurable`]: comparing
/// the publish-epoch against the durable's append-epoch for the same commit
/// proves the group flush ran strictly between append and publish.
pub(super) struct FlushEpochObserver {
    tag: ProviderTag,
    flush_epoch: FlushEpoch,
    /// Node-id → flush-epoch observed when that node was published.
    publish_epoch: Mutex<BTreeMap<u64, u64>>,
}

impl FlushEpochObserver {
    pub(super) fn new(tag: &[u8; 4], flush_epoch: FlushEpoch) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            flush_epoch,
            publish_epoch: Mutex::new(BTreeMap::new()),
        })
    }

    /// The flush-epoch in effect when the node with this id was published.
    pub(super) fn publish_epoch_of(&self, id: u64) -> u64 {
        *self
            .publish_epoch
            .lock()
            .unwrap()
            .get(&id)
            .expect("publish epoch recorded for node id")
    }
}

impl IndexProvider for FlushEpochObserver {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }
    fn read_section(&self, _sub: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }
    fn write_section(&self, _sub: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }
    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        if let Change::NodeCreated { id, .. } = change {
            let epoch = self.flush_epoch.load(Ordering::SeqCst);
            self.publish_epoch.lock().unwrap().insert(id.get(), epoch);
        }
        Ok(())
    }
    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}
