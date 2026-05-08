//! Graph-scoped re-entrancy guard for the provider-fanout phase of commit.
//!
//! Re-entrant writes (a thread calling `SharedGraph::begin_write()` while
//! commit fanout is in progress on this graph) are misuse: the outer commit
//! still holds the write lock and the fanout serializer, so a nested write
//! would either deadlock or recurse indefinitely through the same provider
//! list. The guard is a graph-scoped `AtomicBool` rather than a thread-local
//! so that **any** thread — including a worker thread spawned by the
//! provider's `on_change` — sees the flag and panics before reaching the lock.
//! `begin_write` consults the flag and panics with a clear message; the outer
//! `notify_providers` catches that unwind.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Returns `true` while commit fanout is active on the given graph.
pub(crate) fn in_fanout(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Acquire)
}

/// RAII guard that flips a graph-scoped `AtomicBool` to `true` for the
/// duration of provider fanout. Drop semantics make the reset panic-safe so
/// that a panicking provider cannot leave the flag wedged on.
pub(crate) struct FanoutGuard {
    flag: Arc<AtomicBool>,
}

impl FanoutGuard {
    /// Mark the graph's fanout flag as active. Panics in debug builds if the
    /// flag is already set — that would mean two commits raced their fanouts
    /// on the same graph, which the write lock + allocator mutex are supposed
    /// to forbid.
    pub(crate) fn enter(flag: Arc<AtomicBool>) -> Self {
        let previous = flag.swap(true, Ordering::AcqRel);
        debug_assert!(
            !previous,
            "FanoutGuard::enter called while fanout already active on this graph",
        );
        Self { flag }
    }
}

impl Drop for FanoutGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fanout_guard_sets_and_clears_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(!in_fanout(&flag));
        {
            let _g = FanoutGuard::enter(Arc::clone(&flag));
            assert!(in_fanout(&flag));
        }
        assert!(!in_fanout(&flag));
    }

    #[test]
    fn fanout_guard_clears_flag_on_panic_unwind() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        let result = std::panic::catch_unwind(move || {
            let _g = FanoutGuard::enter(Arc::clone(&flag_clone));
            assert!(in_fanout(&flag_clone));
            panic!("synthetic panic inside fanout");
        });
        assert!(result.is_err());
        assert!(!in_fanout(&flag), "guard's Drop ran on unwind");
    }

    #[test]
    fn fanout_guard_visible_across_threads() {
        let flag = Arc::new(AtomicBool::new(false));
        let _g = FanoutGuard::enter(Arc::clone(&flag));
        let observed = std::thread::scope(|scope| {
            scope
                .spawn({
                    let flag = Arc::clone(&flag);
                    move || in_fanout(&flag)
                })
                .join()
                .expect("worker thread did not panic")
        });
        assert!(observed, "another thread sees the graph-scoped flag");
    }
}
