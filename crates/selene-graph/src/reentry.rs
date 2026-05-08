//! Thread-local re-entrancy guard for the provider-fanout phase of commit.
//!
//! Re-entrant writes (a thread calling `SharedGraph::begin_write()` from
//! inside an `IndexProvider::on_change` callback) are misuse: the outer
//! commit still holds the write lock and the fanout serializer, so a nested
//! write would either deadlock or recurse indefinitely through the same
//! provider list. We detect the misuse here and let `begin_write` panic with
//! a clear message; the outer `notify_providers` catches that unwind.

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
}
