use super::*;

struct ReentrantProvider {
    tag: ProviderTag,
    shared: Mutex<Option<Arc<SharedGraph>>>,
    chained_count: Arc<Mutex<usize>>,
}

impl ReentrantProvider {
    fn new(tag: ProviderTag) -> Self {
        Self {
            tag,
            shared: Mutex::new(None),
            chained_count: Arc::new(Mutex::new(0)),
        }
    }

    fn install_shared(&self, shared: Arc<SharedGraph>) {
        *self.shared.lock() = Some(shared);
    }
}

impl IndexProvider for ReentrantProvider {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        // begin_write() panics on this thread because the FanoutGuard is
        // active. The panic unwinds out of on_change before reaching the
        // chained_count increment.
        let shared = self.shared.lock().take();
        if let Some(shared) = shared {
            let txn = shared.begin_write();
            let _ = txn.commit();
            *self.chained_count.lock() += 1;
        }
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn begin_write_inside_provider_callback_panics_and_is_caught() {
    let provider = Arc::new(ReentrantProvider::new(ProviderTag(*b"REEN")));
    let chained_count = Arc::clone(&provider.chained_count);
    let shared = Arc::new(
        SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
            .build()
            .unwrap(),
    );
    provider.install_shared(Arc::clone(&shared));

    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .expect("create_node ok");
    }
    // Outer commit completes despite the provider's misuse - the panic
    // raised by begin_write inside on_change is caught by
    // notify_providers' catch_unwind boundary.
    let outcome = txn.commit().unwrap();
    assert_eq!(outcome.changes.len(), 1);
    // The increment after begin_write/commit was never reached.
    assert_eq!(
        *chained_count.lock(),
        0,
        "provider's chained mutation must not have completed"
    );
    // After the outer commit returns, the graph's fanout flag is clear
    // and a fresh begin_write succeeds normally.
    let txn = shared.begin_write();
    txn.rollback();
}

/// A provider whose `on_change` panics. Used to verify the engine catches
/// the unwind and continues serving subsequent providers.
struct PanickingProvider {
    tag: ProviderTag,
}

impl IndexProvider for PanickingProvider {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        panic!("synthetic provider panic");
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn provider_panic_does_not_crash_commit_or_block_other_providers() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let shared = SharedGraph::builder(GraphId::new(1))
        .with_provider(Arc::new(PanickingProvider {
            tag: ProviderTag(*b"PANC"),
        }))
        .with_provider(Arc::new(RecordingProvider::new(
            ProviderTag(*b"AFTR"),
            Arc::clone(&seen),
        )))
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .expect("create_node ok")
    };
    let outcome = txn.commit().unwrap();
    assert!(shared.read().is_node_alive(id));
    assert_eq!(outcome.changes.len(), 1);
    // The provider AFTER the panicking one still received the change.
    assert_eq!(seen.lock().len(), 1);
}

/// A provider whose `on_change` blocks for a brief window so a second
/// writer thread can observe that it queues normally on the write lock
/// without panicking - the explicit non-regression for the round-4 P1.
struct SlowProvider {
    tag: ProviderTag,
    hold: std::time::Duration,
}

impl IndexProvider for SlowProvider {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        std::thread::sleep(self.hold);
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn concurrent_writer_does_not_panic_during_other_commits_fanout() {
    // Regression test for the round-4 P1: the previous design had
    // begin_write panic on every concurrent writer while another
    // commit's fanout was running. Concurrent writers must queue on
    // the write lock and proceed normally instead.
    let shared = Arc::new(
        SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(SlowProvider {
                tag: ProviderTag(*b"SLOW"),
                hold: std::time::Duration::from_millis(40),
            }))
            .build()
            .unwrap(),
    );

    let writer_a = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                mutator
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .expect("create_node ok");
            }
            txn.commit().unwrap();
        })
    };

    // Give writer A a head start so it owns the lock and is inside the
    // SlowProvider's on_change.
    thread::sleep(std::time::Duration::from_millis(10));

    // Writer B begins a write while A's fanout is running. It must
    // queue on the write lock, NOT panic.
    let writer_b = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                mutator
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .expect("create_node ok");
            }
            txn.commit().unwrap();
        })
    };

    writer_a.join().expect("writer A finished without panic");
    writer_b.join().expect("writer B finished without panic");
    assert_eq!(shared.read().node_count(), 2);
}

/// A provider whose `provider_tag()` panics only after the build-time
/// uniqueness check has already run successfully. Used to verify the
/// engine short-circuits `on_change` for that provider rather than
/// calling it after a fanout-time tag panic.
struct ConditionallyTagPanickingProvider {
    tag: ProviderTag,
    panic_during_fanout: Arc<std::sync::atomic::AtomicBool>,
    on_change_called: Arc<Mutex<bool>>,
}

impl IndexProvider for ConditionallyTagPanickingProvider {
    fn provider_tag(&self) -> ProviderTag {
        if self
            .panic_during_fanout
            .load(std::sync::atomic::Ordering::Acquire)
        {
            panic!("synthetic provider_tag() panic during fanout");
        }
        self.tag
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        *self.on_change_called.lock() = true;
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn provider_tag_panic_short_circuits_on_change_for_that_provider() {
    let panic_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let on_change_called = Arc::new(Mutex::new(false));
    let other_seen = Arc::new(Mutex::new(Vec::new()));
    let shared = SharedGraph::builder(GraphId::new(1))
        .with_provider(Arc::new(ConditionallyTagPanickingProvider {
            tag: ProviderTag(*b"TPNC"),
            panic_during_fanout: Arc::clone(&panic_flag),
            on_change_called: Arc::clone(&on_change_called),
        }))
        .with_provider(Arc::new(RecordingProvider::new(
            ProviderTag(*b"OTHR"),
            Arc::clone(&other_seen),
        )))
        .build()
        .unwrap();

    // Arm the panic only after the build-time uniqueness check has run.
    panic_flag.store(true, std::sync::atomic::Ordering::Release);

    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .expect("create_node ok");
    }
    txn.commit().unwrap();
    assert!(
        !*on_change_called.lock(),
        "on_change must not run after provider_tag() panicked",
    );
    // The non-panicking provider after the panicking one still ran.
    assert_eq!(other_seen.lock().len(), 1);
}

#[test]
#[cfg(not(miri))]
fn concurrent_writers_notify_provider_for_every_change() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::new(
        SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"CNCR"),
                Arc::clone(&seen),
            )))
            .build()
            .unwrap(),
    );
    let nodes_per_thread = 64;
    thread::scope(|scope| {
        for _ in 0..4 {
            let shared = Arc::clone(&shared);
            scope.spawn(move || {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    for _ in 0..nodes_per_thread {
                        mutator
                            .create_node(LabelSet::new(), PropertyMap::new())
                            .expect("create_node ok");
                    }
                }
                txn.commit().unwrap();
            });
        }
    });
    assert_eq!(shared.read().node_count(), 4 * nodes_per_thread);
    assert_eq!(seen.lock().len(), 4 * nodes_per_thread);
}
