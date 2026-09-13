//! Bound nested `IN [` predicate backtracking without capping unrelated lists.
//!
//! `is_expr` may enter `is_suffix -> IN -> list_lit -> expr` repeatedly. Each
//! still-open wrapper makes pest retry the inner expression cascade when the
//! input is malformed; seventeen wrappers cross the 20-second ASan timeout.
//! Eight active wrappers take about 80 ms under the same harness and remain
//! admitted. This is a program-resource limit, not a grammar rewrite.

use super::{ParserError, point_span};

const MAX_IN_LIST_WRAPPERS: usize = 8;

/// Fixed-size stack of the still-open brackets introduced by `IN`.
#[derive(Default)]
pub(super) struct InListDepth {
    depth: u32,
    wrappers: [u32; MAX_IN_LIST_WRAPPERS],
    active: usize,
}

impl InListDepth {
    pub(super) fn open(&mut self, after_in: bool, offset: usize) -> Result<(), ParserError> {
        self.depth = self.depth.saturating_add(1);
        if after_in {
            if self.active == MAX_IN_LIST_WRAPPERS {
                return Err(ParserError::ComplexityLimitExceeded {
                    limit: MAX_IN_LIST_WRAPPERS as u32,
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
