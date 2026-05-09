//! Transaction-control planner IR rows.

use crate::SourceSpan;

/// Transaction-control operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TxOp {
    /// `START TRANSACTION`.
    Start {
        /// Source span.
        span: SourceSpan,
    },
    /// `COMMIT`.
    Commit {
        /// Source span.
        span: SourceSpan,
    },
    /// `ROLLBACK`.
    Rollback {
        /// Source span.
        span: SourceSpan,
    },
}
