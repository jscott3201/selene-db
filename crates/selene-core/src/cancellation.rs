//! Cooperative cancellation primitives shared by executor and extension crates.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Caller-owned cancellation flag that can be cloned across sessions or threads.
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
