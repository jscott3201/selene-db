use super::*;

pub(super) fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

pub(super) fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-brief2-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn on(max_commits: usize, max_bytes: u64) -> CommitBatching {
    CommitBatching::On {
        max_commits: NonZeroUsize::new(max_commits).unwrap(),
        max_bytes,
    }
}

/// A durable provider that records, in order, every `write_commit` and `flush`
/// event so a test can assert the exact append/fsync interleaving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DurableEvent {
    /// `write_commit` returning this assigned sequence.
    Write(u64),
    /// `flush` returning the high-water sequence.
    Flush(u64),
}

pub(super) struct CountingDurable {
    tag: ProviderTag,
    seq: AtomicU64,
    /// Number of `write_commit` calls before the configured failure (0 = never).
    fail_write_on_call: usize,
    /// Make `flush` fail (after the writes succeed).
    fail_flush: AtomicBool,
    events: Mutex<Vec<DurableEvent>>,
}

impl CountingDurable {
    pub(super) fn new(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            fail_write_on_call: 0,
            fail_flush: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn fail_write_on(tag: &[u8; 4], call: usize) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            fail_write_on_call: call,
            fail_flush: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn fail_flush(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            fail_write_on_call: 0,
            fail_flush: AtomicBool::new(true),
            events: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn events(&self) -> Vec<DurableEvent> {
        self.events.lock().unwrap().clone()
    }

    pub(super) fn write_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, DurableEvent::Write(_)))
            .count()
    }

    pub(super) fn flush_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, DurableEvent::Flush(_)))
            .count()
    }

    /// Largest run of consecutive `write_commit`s between two flushes — i.e. the
    /// largest group-commit batch this durable observed. Because the committer
    /// appends every member of a contiguous run, then issues ONE group flush, the
    /// count of `Write` events between successive `Flush` events equals that run's
    /// batch size. Used to pin the F4 count cap directly (T13b).
    pub(super) fn max_batch_size(&self) -> usize {
        let mut max = 0usize;
        let mut run = 0usize;
        for event in self.events() {
            match event {
                DurableEvent::Write(_) => {
                    run += 1;
                    max = max.max(run);
                }
                DurableEvent::Flush(_) => run = 0,
            }
        }
        max
    }
}

impl DurableProvider for CountingDurable {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn write_commit(
        &self,
        _principal: Option<&Arc<[u8]>>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        let write_calls = self.write_count() + 1;
        if self.fail_write_on_call != 0 && write_calls == self.fail_write_on_call {
            return Err(ProviderError::Inconsistent {
                reason: "synthetic write_commit failure".to_owned(),
            });
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.events.lock().unwrap().push(DurableEvent::Write(seq));
        Ok(seq)
    }

    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        if self.fail_flush.load(Ordering::SeqCst) {
            return Err(ProviderError::Inconsistent {
                reason: "synthetic flush failure".to_owned(),
            });
        }
        let high = self.seq.load(Ordering::SeqCst);
        self.events.lock().unwrap().push(DurableEvent::Flush(high));
        Ok(Some(high))
    }
}

/// Build a SharedGraph with a single synthetic durable provider (no real WAL),
/// under the given batching policy.
pub(super) fn graph_with_durable(
    id: u64,
    durable: Arc<dyn DurableProvider>,
    batching: CommitBatching,
) -> SharedGraph {
    SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(id)),
        Vec::new(),
        vec![durable],
        None,
        None,
        batching,
    )
    .expect("graph builds with synthetic durable provider")
}

/// Build a SharedGraph with a synthetic durable AND a fan-out index provider,
/// under the given batching policy.
pub(super) fn graph_with_durable_and_provider(
    id: u64,
    durable: Arc<dyn DurableProvider>,
    provider: Arc<dyn IndexProvider>,
    batching: CommitBatching,
) -> SharedGraph {
    SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(id)),
        vec![provider],
        vec![durable],
        None,
        None,
        batching,
    )
    .expect("graph builds with synthetic durable + provider")
}
