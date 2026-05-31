//! Thread-local re-entrancy guard for the provider-fanout phase of commit.
//!
//! Re-entrant writes from inside an `IndexProvider::on_change` callback are
//! misuse: a nested write would deadlock or recurse indefinitely. We detect the
//! same-thread case here via a thread-local counter and let `begin_write` panic
//! with a clear message; the `notify_providers` boundary catches that unwind so
//! the commit still completes.
//!
//! ## v1.2 relocation — fanout now runs on the committer thread
//!
//! Before v1.2 fanout ran on the writer (session) thread under the held write
//! lock. Since v1.2 (BRIEF 1) the snapshot publish + provider fan-out run on the
//! single per-graph **committer thread** (`committer.rs`); `WriteTxn::seal`
//! releases the write lock on the session thread before handing the bundle off.
//! This guard is therefore set on the committer thread during fan-out. It stays
//! sound precisely because there is exactly **one** committer: a provider whose
//! `on_change` re-enters `begin_write` does so on the committer thread, where
//! `in_fanout()` is true, so it panics before locking. (The committer itself
//! never holds the write lock — even compaction builds its dense graph on the
//! caller thread and hands the committer a pre-built snapshot — so the panic is
//! about preventing an unbounded re-entrant publish, not a self-deadlock on the
//! write lock.) A second committer would break this thread-local reasoning —
//! the single-committer constraint is load-bearing (v1.2 design §4, §7.7).
//!
//! ## Cross-thread misuse — out of scope
//!
//! A provider whose `on_change` spawns a worker thread, calls `begin_write()`
//! on that worker, AND blocks waiting for the worker (e.g., `JoinHandle::join`,
//! `mpsc::Receiver::recv`) will deadlock. The worker blocks on the held write
//! lock; the outer `on_change` blocks waiting for the worker; the outer commit
//! cannot release the lock until `on_change` returns. This is a circular
//! wait the engine cannot detect without tracing causal thread ancestry.
//!
//! v1.0 contract: `IndexProvider::on_change` MUST NOT initiate a write
//! transaction, directly or indirectly via a spawned thread it then waits on.
//! Cross-thread reentry is documented misuse, not a detectable footgun.
//! Catching the same-thread case here is a courtesy that flags the obvious
//! mistake; cross-thread waits are the provider author's responsibility.

use std::cell::Cell;

thread_local! {
    static FANOUT_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Returns true if this thread is currently inside an
/// [`IndexProvider`](crate::IndexProvider) callback fanout.
pub(crate) fn in_fanout() -> bool {
    FANOUT_DEPTH.with(|cell| cell.get() > 0)
}

/// RAII guard that increments `FANOUT_DEPTH` for the duration of provider
/// fanout. Drop semantics make the decrement panic-safe so that a panicking
/// provider cannot leave the counter wedged above zero.
pub(crate) struct FanoutGuard {
    _private: (),
}

impl FanoutGuard {
    pub(crate) fn enter() -> Self {
        FANOUT_DEPTH.with(|cell| cell.set(cell.get().saturating_add(1)));
        Self { _private: () }
    }
}

impl Drop for FanoutGuard {
    fn drop(&mut self) {
        FANOUT_DEPTH.with(|cell| {
            let value = cell.get();
            debug_assert!(value > 0, "FanoutGuard dropped without matching enter");
            cell.set(value.saturating_sub(1));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fanout_guard_increments_and_decrements() {
        assert!(!in_fanout());
        {
            let _g = FanoutGuard::enter();
            assert!(in_fanout());
            {
                let _h = FanoutGuard::enter();
                assert!(in_fanout());
            }
            assert!(in_fanout());
        }
        assert!(!in_fanout());
    }

    #[test]
    fn fanout_guard_decrements_on_panic_unwind() {
        let result = std::panic::catch_unwind(|| {
            let _g = FanoutGuard::enter();
            assert!(in_fanout());
            panic!("synthetic panic inside fanout");
        });
        assert!(result.is_err());
        assert!(!in_fanout(), "guard's Drop ran on unwind");
    }

    #[test]
    fn fanout_guard_does_not_leak_into_other_threads() {
        let _g = FanoutGuard::enter();
        assert!(in_fanout());
        let observed = std::thread::scope(|scope| {
            scope.spawn(in_fanout).join().expect("worker did not panic")
        });
        assert!(
            !observed,
            "thread-local guard is intentionally not visible to other threads",
        );
    }
}
