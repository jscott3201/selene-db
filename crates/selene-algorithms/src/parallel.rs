//! Parallel execution policy shared by algorithm implementations.
//!
//! The public surface is stable before every algorithm consumes it so parallel
//! execution can expand without reshaping caller configuration repeatedly.

use std::num::NonZeroUsize;

/// Upper bound for explicit Rayon thread counts requested by a caller.
///
/// This cap is a denial-of-service guard: a [`Parallelism::Threads`] policy
/// spins up a dedicated Rayon pool of `n` OS threads (the crate-internal
/// `ParallelRunner`), so an unbounded `n` sized by an untrusted GQL argument
/// would let a single `CALL` exhaust the host's thread budget (see
/// `feedback_mem_forget_leak_dos`). `1024` matches the historical
/// adapter-side cap that previously lived in `selene-algorithms-pack`; it is
/// large enough to never constrain a legitimate request yet bounds the worst
/// case. The cap is enforced by [`Parallelism::from_thread_count`].
pub const MAX_PARALLELISM_THREADS: usize = 1024;

/// Error returned by [`Parallelism::from_thread_count`] for an out-of-range
/// thread count.
///
/// The two variants are kept distinct so callers (e.g. the GQL procedure
/// registry) can render the historical adapter-specific error messages
/// verbatim: a too-large `u64`/`usize` argument versus a request that exceeds
/// [`MAX_PARALLELISM_THREADS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelismCapError {
    /// The requested thread count exceeds [`MAX_PARALLELISM_THREADS`].
    ///
    /// `requested` is the value the caller asked for; the effective cap is
    /// [`MAX_PARALLELISM_THREADS`].
    ExceedsCap {
        /// The thread count the caller requested.
        requested: usize,
    },
}

/// Requested parallel execution policy for graph algorithms.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parallelism {
    /// Force single-threaded execution.
    Sequential,
    /// Use the current Rayon pool size, or the global pool size outside Rayon.
    #[default]
    Auto,
    /// Use an explicit non-zero thread count.
    Threads(NonZeroUsize),
}

impl Parallelism {
    /// Build a parallelism policy from an explicit thread count, enforcing the
    /// [`MAX_PARALLELISM_THREADS`] DoS cap.
    ///
    /// - `0` → [`Parallelism::Sequential`] (a zero-thread pool is meaningless;
    ///   the historical adapter mapped `0` to sequential execution).
    /// - `1..=MAX_PARALLELISM_THREADS` → [`Parallelism::Threads`].
    /// - `> MAX_PARALLELISM_THREADS` → [`ParallelismCapError::ExceedsCap`].
    ///
    /// This is the single home for the thread-count cap policy that previously
    /// lived in the `selene-algorithms-pack` adapter; the GQL procedure
    /// registry calls it so the cap is identical across every algorithm.
    pub fn from_thread_count(threads: usize) -> Result<Self, ParallelismCapError> {
        if threads == 0 {
            return Ok(Self::Sequential);
        }
        if threads > MAX_PARALLELISM_THREADS {
            return Err(ParallelismCapError::ExceedsCap { requested: threads });
        }
        Ok(Self::Threads(
            NonZeroUsize::new(threads).expect("threads is non-zero after the zero check above"),
        ))
    }

    /// Return the effective thread count for this policy.
    #[must_use]
    pub fn effective_threads(self) -> usize {
        match self {
            Self::Sequential => 1,
            Self::Auto => rayon::current_num_threads(),
            Self::Threads(n) => n.get(),
        }
    }
}

pub(crate) struct ParallelRunner {
    pool: Option<rayon::ThreadPool>,
}

#[derive(Debug, thiserror::Error)]
#[error("rayon thread pool build failed: {0}")]
pub(crate) struct ParallelRunnerError(#[from] rayon::ThreadPoolBuildError);

impl ParallelRunner {
    pub(crate) fn new(parallelism: Parallelism) -> Result<Self, ParallelRunnerError> {
        let pool = match parallelism {
            Parallelism::Threads(threads) => Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads.get())
                    .build()?,
            ),
            Parallelism::Sequential | Parallelism::Auto => None,
        };
        Ok(Self { pool })
    }

    pub(crate) fn install<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        match &self.pool {
            Some(pool) => pool.install(f),
            None => f(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    use super::*;

    #[test]
    fn parallelism_default_is_auto() {
        assert_eq!(Parallelism::default(), Parallelism::Auto);
    }

    #[test]
    fn parallelism_effective_threads_sequential() {
        assert_eq!(Parallelism::Sequential.effective_threads(), 1);
    }

    #[test]
    fn parallelism_effective_threads_threads_n() {
        let policy = Parallelism::Threads(NonZeroUsize::new(4).unwrap());
        assert_eq!(policy.effective_threads(), 4);
    }

    #[test]
    fn parallelism_effective_threads_auto_positive() {
        assert!(Parallelism::Auto.effective_threads() >= 1);
    }

    #[test]
    fn parallelism_derives_intact() {
        let a = Parallelism::Threads(NonZeroUsize::new(2).unwrap());
        let b = a;
        assert_eq!(a, b);

        let mut hasher = DefaultHasher::new();
        a.hash(&mut hasher);
        assert_ne!(hasher.finish(), 0);
    }

    #[test]
    fn from_thread_count_zero_is_sequential() {
        assert_eq!(
            Parallelism::from_thread_count(0),
            Ok(Parallelism::Sequential)
        );
    }

    #[test]
    fn from_thread_count_one_is_single_thread_pool() {
        assert_eq!(
            Parallelism::from_thread_count(1),
            Ok(Parallelism::Threads(NonZeroUsize::new(1).unwrap()))
        );
    }

    #[test]
    fn from_thread_count_positive_is_threads() {
        assert_eq!(
            Parallelism::from_thread_count(4),
            Ok(Parallelism::Threads(NonZeroUsize::new(4).unwrap()))
        );
    }

    #[test]
    fn from_thread_count_at_cap_is_accepted() {
        assert_eq!(
            Parallelism::from_thread_count(MAX_PARALLELISM_THREADS),
            Ok(Parallelism::Threads(
                NonZeroUsize::new(MAX_PARALLELISM_THREADS).unwrap()
            ))
        );
    }

    #[test]
    fn from_thread_count_above_cap_is_rejected() {
        assert_eq!(
            Parallelism::from_thread_count(MAX_PARALLELISM_THREADS + 1),
            Err(ParallelismCapError::ExceedsCap {
                requested: MAX_PARALLELISM_THREADS + 1,
            })
        );
    }
}
