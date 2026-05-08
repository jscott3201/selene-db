//! Per-parse interner admission budget.

use selene_core::{IStr, intern_with_admission, lookup};

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

    fn charge(&mut self, span: SourceSpan) -> Result<(), ParserError> {
        self.admitted = self.admitted.saturating_add(1);
        if self.admitted > self.limit {
            return Err(ParserError::InternerBudgetExceeded {
                limit: self.limit,
                span,
            });
        }
        Ok(())
    }

    pub(super) fn intern_str(
        &mut self,
        value: &str,
        span: SourceSpan,
        kind: &'static str,
    ) -> Result<IStr, ParserError> {
        // Already interned: free admission, no global growth, no budget charge.
        if let Some(existing) = lookup(value) {
            return Ok(existing);
        }
        // Would be a new admission. Charge the local budget BEFORE the global
        // intern call so an over-budget parse never permanently grows the
        // process-wide pool.
        self.charge(span)?;
        let (interned, _was_new) = intern_with_admission(value).map_err(|error| {
            ParserError::syntax(
                format!("could not intern {kind}: {error}"),
                span,
                Some("interner cap may be exhausted".into()),
            )
        })?;
        Ok(interned)
    }
}
