//! Optimizer rule traits and rewrite results.

use crate::plan::{ExecutionPlan, optimize::OptimizeContext};

/// One infallible optimizer rewrite rule.
pub trait Rule: Send + Sync + 'static {
    /// Stable rule name used by tests and future metrics.
    fn name(&self) -> &'static str;

    /// Rewrite a plan once.
    fn rewrite(&self, plan: ExecutionPlan, ctx: &OptimizeContext<'_>)
    -> Transformed<ExecutionPlan>;
}

/// A rewritten value paired with a change flag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transformed<T> {
    /// Rewritten value.
    pub plan: T,
    /// Whether the rewrite changed the value.
    pub changed: bool,
}

impl<T> Transformed<T> {
    /// Wrap an unchanged value.
    #[must_use]
    pub const fn unchanged(plan: T) -> Self {
        Self {
            plan,
            changed: false,
        }
    }

    /// Wrap a changed value.
    #[must_use]
    pub const fn changed(plan: T) -> Self {
        Self {
            plan,
            changed: true,
        }
    }

    /// Map the contained value while preserving the change flag.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Transformed<U> {
        Transformed {
            plan: f(self.plan),
            changed: self.changed,
        }
    }
}
