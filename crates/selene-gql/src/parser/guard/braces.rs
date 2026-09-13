//! Bound the demonstrated bare nested-query backtracking path, not all braces.
//!
//! `query_specification` retries `query_pipeline` through composite, chained and
//! plain alternatives. Each bare `{ { ... } }` wrapper multiplies failed work;
//! F06-QUAL-04 reached the 20-second ASan timeout with only thirteen openers.
//! Seven active brace-to-brace transitions admit eight bare levels (155 ms for
//! the malformed ASan probe), well below that exponential cliff. This is a
//! program-resource limit, not a grammar rewrite. Record fields and `EXISTS`
//! bodies do not begin with `{`, so their existing nesting limits are unchanged.

use super::{ParserError, point_span};

const MAX_BARE_QUERY_WRAPPERS: usize = 7;

/// Fixed-size stack of the still-open braces that begin a bare nested query.
#[derive(Default)]
pub(super) struct BareQueryDepth {
    depth: u32,
    wrappers: [u32; MAX_BARE_QUERY_WRAPPERS],
    active: usize,
}

impl BareQueryDepth {
    pub(super) fn open(&mut self, after_brace: bool, offset: usize) -> Result<(), ParserError> {
        self.depth = self.depth.saturating_add(1);
        if after_brace {
            if self.active == MAX_BARE_QUERY_WRAPPERS {
                return Err(ParserError::ComplexityLimitExceeded {
                    limit: MAX_BARE_QUERY_WRAPPERS as u32,
                    span: point_span(offset),
                });
            }
            self.wrappers[self.active] = self.depth;
            self.active += 1;
        }
        Ok(())
    }

    pub(super) fn close(&mut self) {
        if self.active > 0 && self.wrappers[self.active - 1] == self.depth {
            self.active -= 1;
        }
        self.depth = self.depth.saturating_sub(1);
    }
}
