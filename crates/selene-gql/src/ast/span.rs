//! Source span tracking for parsed GQL input.

/// Byte span in the original query source.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub struct SourceSpan {
    /// Byte offset from the start of the source string.
    pub byte_offset: u32,
    /// Span length in bytes.
    pub byte_len: u32,
}

impl SourceSpan {
    /// Construct a span from byte offset and byte length.
    #[must_use]
    pub const fn new(byte_offset: u32, byte_len: u32) -> Self {
        Self {
            byte_offset,
            byte_len,
        }
    }

    /// Return the first byte offset after this span.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.byte_offset.saturating_add(self.byte_len)
    }

    /// Merge two spans into one span covering both.
    #[must_use]
    pub fn merge(left: Self, right: Self) -> Self {
        let start = left.byte_offset.min(right.byte_offset);
        let end = left.end().max(right.end());
        Self::new(start, end.saturating_sub(start))
    }

    pub(crate) fn from_pest(span: pest::Span<'_>) -> Self {
        let start = u32::try_from(span.start()).unwrap_or(u32::MAX);
        let end = u32::try_from(span.end()).unwrap_or(u32::MAX);
        Self::new(start, end.saturating_sub(start))
    }
}

impl From<SourceSpan> for miette::SourceSpan {
    fn from(span: SourceSpan) -> Self {
        miette::SourceSpan::new((span.byte_offset as usize).into(), span.byte_len as usize)
    }
}
