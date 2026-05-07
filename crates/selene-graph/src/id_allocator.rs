#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Per-graph ID allocator skeleton for `selene-graph`.
//!
//! D11 settles real ID allocation under the graph write-lock. `alloc_node_id`
//! and `alloc_edge_id` are called while a `WriteTxn` holds that lock, so aborted
//! transaction allocations are not rolled back; they become permanent holes in
//! the dense monotonic sequence. See `_spec/02-data-model.md` §4 for the
//! identity rule and `_spec/03-property-graph-and-concurrency.md` §6 for the
//! transaction lifecycle. Atomics are used so recovery can restore checkpoints
//! without widening this skeleton into a runtime concurrency design.

use std::sync::atomic::AtomicU64;

/// Per-graph allocator for node and edge identifiers.
///
/// Allocation starts at one because ID zero is reserved as a tombstone sentinel
/// in spec 02 §4. Calls are made under the D10 write-lock; the atomics are not
/// a multi-writer permission slip.
pub struct IdAllocator {
    next_node_id: AtomicU64,
    next_edge_id: AtomicU64,
}

impl IdAllocator {
    /// Create an allocator at the v1.0 initial checkpoint.
    ///
    /// The first allocated node and edge IDs are both `1`. See spec 02 §4.
    pub fn new() -> Self {
        unimplemented!("M2 work")
    }

    /// Allocate the next real node ID.
    ///
    /// The caller must hold the graph write-lock. If the surrounding
    /// transaction aborts, this ID is still consumed as a hole per D11.
    pub fn alloc_node_id(&self) -> NodeId {
        unimplemented!("M2 work")
    }

    /// Allocate the next real edge ID.
    ///
    /// The caller must hold the graph write-lock. If the surrounding
    /// transaction aborts, this ID is still consumed as a hole per D11.
    pub fn alloc_edge_id(&self) -> EdgeId {
        unimplemented!("M2 work")
    }

    /// Capture the allocator high-water marks for snapshot publication.
    ///
    /// Committed checkpoints preserve abort holes that advanced the counters.
    /// See `_spec/03` §6.2 for the commit boundary.
    pub fn snapshot(&self) -> IdCheckpoint {
        unimplemented!("M2 work")
    }

    /// Restore allocator high-water marks during recovery.
    ///
    /// This is the recovery-only path; normal writes advance IDs through
    /// [`Self::alloc_node_id`] and [`Self::alloc_edge_id`] under the write-lock.
    pub fn restore(&self, _ckpt: IdCheckpoint) {
        unimplemented!("M2 work")
    }
}

/// Durable allocator checkpoint stored in graph metadata.
///
/// The fields represent the next IDs to allocate, not the most recent IDs
/// allocated. See spec 02 §4 for the monotonic rule.
pub struct IdCheckpoint {
    /// Next node ID to allocate.
    pub next_node_id: u64,
    /// Next edge ID to allocate.
    pub next_edge_id: u64,
}

/// Placeholder node ID; the final type lives in `selene-core`.
pub struct NodeId(u64);

/// Placeholder edge ID; the final type lives in `selene-core`.
pub struct EdgeId(u64);
