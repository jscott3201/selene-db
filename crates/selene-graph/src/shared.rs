//! Shared graph wrapper implementing lock-free reads and serialized writes.

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};

use selene_core::GraphId;

use crate::graph::SeleneGraph;
use crate::id_allocator::IdAllocator;
use crate::write_txn::WriteTxn;

/// Per-graph shared runtime state.
pub struct SharedGraph {
    shared: Arc<RwLock<SeleneGraph>>,
    snapshot: Arc<ArcSwap<SeleneGraph>>,
    allocator: Arc<Mutex<IdAllocator>>,
}

impl SharedGraph {
    /// Construct an empty shared graph.
    #[must_use]
    pub fn new(graph_id: GraphId) -> Self {
        Self::from_graph(SeleneGraph::new(graph_id))
    }

    /// Construct shared state from a pre-built graph snapshot.
    #[must_use]
    pub fn from_graph(graph: SeleneGraph) -> Self {
        let allocator = IdAllocator::from_meta(&graph.meta);
        Self {
            shared: Arc::new(RwLock::new(graph.clone())),
            snapshot: Arc::new(ArcSwap::new(Arc::new(graph))),
            allocator: Arc::new(Mutex::new(allocator)),
        }
    }

    /// Load the current immutable snapshot without taking the write lock.
    #[must_use]
    pub fn read(&self) -> Arc<SeleneGraph> {
        self.snapshot.load_full()
    }

    /// Begin a write transaction by acquiring the single graph write lock.
    #[must_use]
    pub fn begin_write(&self) -> WriteTxn<'_> {
        WriteTxn::new(
            self.shared.write(),
            Arc::clone(&self.snapshot),
            self.allocator.lock(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn new_initial_state_is_empty() {
        let shared = SharedGraph::new(GraphId::new(1));
        assert_eq!(shared.read().node_count(), 0);
    }

    #[test]
    fn read_is_lock_free_concurrent_with_another_reader() {
        let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
        thread::scope(|scope| {
            for _ in 0..8 {
                let shared = Arc::clone(&shared);
                scope.spawn(move || {
                    for _ in 0..10_000 {
                        assert_eq!(shared.read().meta.graph_id, GraphId::new(1));
                    }
                });
            }
        });
    }

    #[test]
    fn read_during_write_lock_held_does_not_block() {
        let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
        thread::scope(|scope| {
            let writer_graph = Arc::clone(&shared);
            let writer = scope.spawn(move || {
                let _txn = writer_graph.begin_write();
                thread::sleep(Duration::from_millis(75));
            });
            thread::sleep(Duration::from_millis(10));
            let start = Instant::now();
            for _ in 0..4 {
                let reader_graph = Arc::clone(&shared);
                scope.spawn(move || {
                    for _ in 0..100 {
                        assert_eq!(reader_graph.read().node_count(), 0);
                    }
                });
            }
            writer.join().unwrap();
            assert!(start.elapsed() < Duration::from_millis(500));
        });
    }
}
