//! Cooperative cancellation primitives shared by the executor and algorithm crates.
//!
//! Cancellation is cooperative: callers request cancellation through a
//! [`CancellationToken`], and long-running executor or algorithm loops observe
//! it at explicit checkpoints. Work between checkpoints is allowed to finish
//! before the cancellation surfaces.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Caller-owned cancellation flag that can be cloned across sessions or threads.
///
/// Clones share the same underlying flag, so a host can keep one handle and
/// pass another into `selene-gql` session builders. Calling [`Self::cancel`]
/// requests cancellation; it does not interrupt running Rust code
/// preemptively.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Construct a new token in the non-cancelled state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cooperative cancellation for all holders of this token.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Return true when cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Cancellation outcome reported by cooperative checkpoint calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationCause {
    /// The caller-owned token was cancelled.
    Cancelled,
    /// The statement deadline passed before the checkpoint.
    Timeout {
        /// Wall-clock duration since the deadline elapsed.
        elapsed: Duration,
    },
}

/// Cheap composite checker passed into hot loops that cannot depend on `selene-gql`.
///
/// The checker combines an optional cancellation token with an optional
/// absolute deadline. It is intentionally `Copy` so callers can pass it into
/// nested loops and algorithm hot loops without allocation.
#[derive(Clone, Copy, Debug)]
pub struct CancellationChecker<'a> {
    token: Option<&'a CancellationToken>,
    deadline: Option<Instant>,
}

impl<'a> CancellationChecker<'a> {
    /// Construct a checker from an optional token and optional deadline.
    #[must_use]
    pub const fn new(token: Option<&'a CancellationToken>, deadline: Option<Instant>) -> Self {
        Self { token, deadline }
    }

    /// Construct a checker that never cancels or times out.
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            token: None,
            deadline: None,
        }
    }

    /// Return true when this checker has no token or deadline to inspect.
    #[must_use]
    #[inline(always)]
    pub const fn is_disabled(&self) -> bool {
        self.token.is_none() && self.deadline.is_none()
    }

    /// Return the first cancellation cause observed at this checkpoint.
    ///
    /// Token cancellation wins over deadline timeout so explicit caller
    /// cancellation is not reported as a timeout when both are true.
    #[inline]
    pub fn check(&self) -> Result<(), CancellationCause> {
        if self.token.is_some_and(CancellationToken::is_cancelled) {
            return Err(CancellationCause::Cancelled);
        }
        if let Some(deadline) = self.deadline {
            let now = Instant::now();
            if now >= deadline {
                return Err(CancellationCause::Timeout {
                    elapsed: now.duration_since(deadline),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_checker_never_trips() {
        let checker = CancellationChecker::disabled();
        assert!(checker.is_disabled());
        assert_eq!(checker.check(), Ok(()));
    }

    #[test]
    fn checker_with_token_is_not_disabled() {
        let token = CancellationToken::new();
        let checker = CancellationChecker::new(Some(&token), None);
        assert!(!checker.is_disabled());
    }

    #[test]
    fn checker_with_deadline_is_not_disabled() {
        let deadline = Instant::now();
        let checker = CancellationChecker::new(None, Some(deadline));
        assert!(!checker.is_disabled());
    }

    #[test]
    fn token_wins_over_deadline_when_both_tripped() {
        // CORE-09: both a cancelled token AND an elapsed deadline are set. The
        // checker must report Cancelled (explicit caller intent), not Timeout.
        let token = CancellationToken::new();
        token.cancel();
        let elapsed_deadline = Instant::now() - Duration::from_secs(1);
        let checker = CancellationChecker::new(Some(&token), Some(elapsed_deadline));
        assert_eq!(checker.check(), Err(CancellationCause::Cancelled));
    }

    #[test]
    fn deadline_reported_when_only_deadline_tripped() {
        let elapsed_deadline = Instant::now() - Duration::from_secs(1);
        let checker = CancellationChecker::new(None, Some(elapsed_deadline));
        assert!(matches!(
            checker.check(),
            Err(CancellationCause::Timeout { .. })
        ));
    }

    #[test]
    fn live_token_with_future_deadline_passes() {
        let token = CancellationToken::new();
        let future_deadline = Instant::now() + Duration::from_secs(3600);
        let checker = CancellationChecker::new(Some(&token), Some(future_deadline));
        assert_eq!(checker.check(), Ok(()));
    }
}
