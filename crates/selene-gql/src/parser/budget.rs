//! Per-parse interner admission budget.

use selene_core::{AdmissionError, IStr, intern_atomic_admit};

use crate::{SourceSpan, error::ParserError};

/// Maximum distinct new strings a single parse may admit.
pub(super) const MAX_NEW_ADMISSIONS_PER_PARSE: u32 = 8_192;

#[derive(Debug)]
pub(super) struct InternerBudget {
    limit: u32,
    admitted: u32,
}

impl InternerBudget {
    pub(super) const fn new(limit: u32) -> Self {
        Self { limit, admitted: 0 }
    }

    pub(super) fn intern_str(
        &mut self,
        value: &str,
        span: SourceSpan,
        kind: &'static str,
    ) -> Result<IStr, ParserError> {
        // The closure runs only when this call would be a genuine new
        // admission from this thread's vantage, under the same lock that
        // gates the global cap check and insertion. If the closure rejects,
        // no global insertion happens; if it accepts, the local counter and
        // the global pool advance atomically. This makes the per-parse
        // budget race-free against other threads admitting the same string:
        // the closure is never invoked when another thread has already
        // interned `value`, so we never over-charge for redundant work.
        intern_atomic_admit(value, || {
            if self.admitted >= self.limit {
                return Err(());
            }
            self.admitted += 1;
            Ok(())
        })
        .map_err(|error| match error {
            AdmissionError::Cap(cap) => ParserError::syntax(
                format!("could not intern {kind}: {cap}"),
                span,
                Some("interner cap may be exhausted".into()),
            ),
            AdmissionError::Rejected => ParserError::InternerBudgetExceeded {
                limit: self.limit,
                span,
            },
            // AdmissionError is #[non_exhaustive] so cross-crate matches
            // need this arm. Treat any future variant conservatively as a
            // syntax error rather than silently mapping it to the budget
            // path, where the limit field would be misleading.
            other => ParserError::syntax(
                format!("could not intern {kind}: {other}"),
                span,
                None,
            ),
        })
    }
}
