//! Session-control planner IR rows (ISO/IEC 39075:2024 section 7).

use selene_core::DbString;

use crate::{GqlType, SessionSetGraphTarget, SourceSpan, ValueExpr};

/// Session-control operation lowered from a `SESSION` command.
#[derive(Clone, Debug)]
pub enum SessionOp {
    /// `SESSION SET VALUE <param> [<type>] = <value expression>` (ISO feature GS03).
    ///
    /// The value is evaluated against an empty binding row at execution time
    /// (restricted to a `<value specification>`; see GS14 rationale).
    SetValue {
        /// Database-string parameter name without the leading `$`.
        param: DbString,
        /// Optional declared type for the target session parameter.
        declared_type: Option<GqlType>,
        /// Value expression bound to the parameter.
        value: Box<ValueExpr>,
        /// When set, leave an existing binding untouched (`IF NOT EXISTS`).
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION SET TIME ZONE <time zone string>` (ISO feature GS15).
    SetTimeZone {
        /// Decoded IANA region name or fixed-offset string.
        zone: String,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION SET [PROPERTY] GRAPH <current graph>` (ISO/IEC 39075:2024 section 7.1).
    SetGraph {
        /// Current-graph expression selected by the command.
        target: SessionSetGraphTarget,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION RESET [ALL] CHARACTERISTICS` / bare `SESSION RESET` (GS04).
    ResetAllCharacteristics {
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION RESET [ALL] PARAMETERS` (ISO feature GS08).
    ResetParameters {
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION RESET TIME ZONE` (ISO feature GS07).
    ResetTimeZone {
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION RESET [PARAMETER] <param>` (ISO feature GS16).
    ResetParameter {
        /// Database-string parameter name without the leading `$`.
        param: DbString,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION CLOSE` (ISO/IEC 39075:2024 section 7.3).
    Close {
        /// Source span.
        span: SourceSpan,
    },
}
