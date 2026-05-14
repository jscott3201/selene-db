//! Parallel execution policy shared by future algorithm implementations.
//!
//! BRIEF-82 introduces the public surface before any algorithm consumes it so
//! M12 follow-up briefs can add parallel execution without reshaping caller
//! configuration repeatedly.

use std::num::NonZeroUsize;

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
}
