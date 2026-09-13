//! Retain the shared query/IN resource limit after removing their PEG retries.
//!
//! The grammar now factors query pipelines and excludes a failed direct list
//! from the comparison alternative. This makes it safe to defer this particular
//! limit until the single parse resolves ambiguous identifier/string quotes.
//! The initial scan records a candidate error for malformed input; the parsed
//! scan uses authoritative quote spans. Neither scan can disable the independent
//! delimiter, bare-query or zero-delimiter recursion limits.
//! As in the original IN-list counter, `FOR x IN [` is conservatively charged;
//! distinguishing that statement binding would require additional token state.

use super::{ParserError, point_span};

const MAX_RETRY_FRAMES: usize = 8;

/// Fixed-size stack of retry frames, identified by delimiter kind and depth.
#[derive(Default)]
pub(super) struct BacktrackingDepth {
    depths: [u32; 2],
    wrappers: [(u8, u32); MAX_RETRY_FRAMES],
    active: usize,
    pub(super) error: Option<ParserError>,
}

impl BacktrackingDepth {
    pub(super) fn open(&mut self, delimiter: u8, retries: bool, offset: usize) {
        let depth = &mut self.depths[usize::from(delimiter == b'{')];
        *depth = depth.saturating_add(1);
        if self.error.is_some() {
            return;
        }
        if retries {
            if self.active == MAX_RETRY_FRAMES {
                self.error = Some(ParserError::ComplexityLimitExceeded {
                    limit: MAX_RETRY_FRAMES as u32,
                    span: point_span(offset),
                });
                return;
            }
            self.wrappers[self.active] = (delimiter, *depth);
            self.active += 1;
        }
    }

    pub(super) fn close(&mut self, delimiter: u8) {
        let depth = &mut self.depths[usize::from(delimiter == b'{')];
        if self.active > 0 && self.wrappers[self.active - 1] == (delimiter, *depth) {
            self.active -= 1;
        }
        *depth = depth.saturating_sub(1);
    }
}
