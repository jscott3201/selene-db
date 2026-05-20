//! Parser error types and GQLSTATUS mappings.

use std::fmt;

use selene_core::feature_register::FeatureId;

use crate::ast::span::SourceSpan;

/// Five-character ISO GQLSTATUS code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GqlStatus([u8; 5]);

impl GqlStatus {
    /// Maps to GQLSTATUS 42001 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const SYNTAX_ERROR: Self = Self(*b"42001");
    /// Maps to GQLSTATUS 42N01, a selene-db implementation-defined subclass
    /// under standard class 42 per ISO/IEC 39075:2024 section 23.1.
    pub const FEATURE_NOT_SUPPORTED: Self = Self(*b"42N01");
    /// Maps to GQLSTATUS 5GQL1, a selene-db implementation-defined class per
    /// ISO/IEC 39075:2024 section 23.1.
    pub const PROGRAM_LIMIT_EXCEEDED: Self = Self(*b"5GQL1");
    /// Maps to GQLSTATUS 5GQL2, a selene-db implementation-defined class per
    /// ISO/IEC 39075:2024 section 23.1.
    pub const OPERATION_CANCELLED: Self = Self(*b"5GQL2");
    /// Maps to GQLSTATUS 5GQL3, a selene-db implementation-defined class per
    /// ISO/IEC 39075:2024 section 23.1.
    pub const DEADLINE_EXCEEDED: Self = Self(*b"5GQL3");
    /// Maps to GQLSTATUS 42N03, a selene-db implementation-defined subclass
    /// under standard class 42 per ISO/IEC 39075:2024 section 23.1.
    pub const UNDEFINED_REFERENCE: Self = Self(*b"42N03");
    /// Maps to GQLSTATUS 42002 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const INVALID_REFERENCE: Self = Self(*b"42002");
    /// Maps to GQLSTATUS 42N10, a selene-db implementation-defined subclass
    /// under standard class 42 per ISO/IEC 39075:2024 section 23.1.
    pub const DUPLICATE_OBJECT: Self = Self(*b"42N10");
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const DATATYPE_MISMATCH: Self = Self(*b"22G03");
    /// Maps to GQLSTATUS 22000 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const DATA_EXCEPTION: Self = Self(*b"22000");
    /// Maps to GQLSTATUS 25000 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const INVALID_TRANSACTION_STATE: Self = Self(*b"25000");
    /// Maps to GQLSTATUS 25G02 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const INVALID_TRANSACTION_STATE_MIXING: Self = Self(*b"25G02");
    /// Maps to GQLSTATUS 25N02, a selene-db implementation-defined subclass
    /// under standard class 25 per ISO/IEC 39075:2024 section 23.1.
    pub const IN_FAILED_TRANSACTION: Self = Self(*b"25N02");
    /// Maps to GQLSTATUS 42N04, a selene-db implementation-defined subclass
    /// under standard class 42 per ISO/IEC 39075:2024 section 23.1.
    pub const UNKNOWN_PROCEDURE: Self = Self(*b"42N04");
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    pub const INVALID_PROCEDURE_ARGUMENT: Self = Self(*b"22G03");
    /// Maps to GQLSTATUS 42N28, a selene-db implementation-defined subclass
    /// under standard class 42 per ISO/IEC 39075:2024 section 23.1.
    pub const CAPABILITY_VIOLATION: Self = Self(*b"42N28");
    /// Maps to GQLSTATUS 5GQL0, a selene-db implementation-defined class per
    /// ISO/IEC 39075:2024 section 23.1. Specific executor diagnostics carry
    /// detail tags under this single public class.
    pub const IMPLEMENTATION_DEFINED_ERROR: Self = Self(*b"5GQL0");

    /// Return this status as its 5-character string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("5GQL0")
    }

    /// Return this status's two-character class code.
    #[must_use]
    pub const fn class(&self) -> [u8; 2] {
        [self.0[0], self.0[1]]
    }
}

impl fmt::Display for GqlStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// GQL parser and flagger error.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ParserError {
    /// Source text did not parse as supported GQL syntax.
    #[error("{message}")]
    #[diagnostic(code(SLENE_GQL_42001))]
    SyntaxError {
        /// ISO GQLSTATUS code.
        status: GqlStatus,
        /// Human-readable diagnostic message.
        message: String,
        /// Source span for the parse failure.
        #[label("here")]
        span: SourceSpan,
        /// Optional repair hint.
        #[help]
        hint: Option<String>,
    },

    /// Parsed syntax requires a feature outside the current support set.
    #[error("feature not supported: {} ({display_name})", feature_id.as_str())]
    #[diagnostic(code(SLENE_GQL_42N01))]
    UnsupportedFeature {
        /// ISO feature identifier.
        feature_id: FeatureId,
        /// Human-readable feature name.
        display_name: &'static str,
        /// Source span requiring the feature.
        #[label("requires feature")]
        span: SourceSpan,
        /// Static hint for enabling or avoiding the feature.
        #[help]
        hint: &'static str,
    },

    /// Query introduced more distinct interner admissions than the parser cap allows.
    #[error("interner-admission budget exceeded ({limit})")]
    #[diagnostic(
        code(SLENE_GQL_5GQL1),
        help("queries are bounded to 8192 distinct new interner admissions per parse")
    )]
    InternerBudgetExceeded {
        /// Distinct admission limit.
        limit: u32,
        /// Source span that crossed the limit.
        #[label("introduces too many new interned strings")]
        span: SourceSpan,
    },

    /// Query nesting exceeded the parser's recursion cap.
    #[error("parser nesting limit exceeded ({limit})")]
    #[diagnostic(
        code(SLENE_GQL_5GQL1),
        help("queries are bounded to 64 nested grouping, list, and record delimiters")
    )]
    NestingLimitExceeded {
        /// Maximum admitted syntactic nesting depth.
        limit: u32,
        /// Source span that crossed the limit.
        #[label("exceeds parser nesting limit")]
        span: SourceSpan,
    },

    /// Source parsed at the grammar level, but no AST builder is implemented yet.
    ///
    /// Distinct from [`Self::SyntaxError`] (parse failed) and
    /// [`Self::UnsupportedFeature`] (specific ISO feature not in the claim list).
    /// This variant covers grammar surfaces selene-db will support but whose
    /// builders land in a later brief.
    #[error("not implemented: {message}")]
    #[diagnostic(code(SLENE_GQL_42N01))]
    NotImplemented {
        /// Human-readable description of the missing capability.
        message: String,
        /// Source span requiring the missing capability.
        #[label("not implemented yet")]
        span: SourceSpan,
        /// Pointer to the brief or milestone that lands the capability.
        #[help]
        hint: Option<String>,
    },
}

impl ParserError {
    /// Map this error to its 5-character ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::SyntaxError { status, .. } => *status,
            Self::UnsupportedFeature { .. } | Self::NotImplemented { .. } => {
                GqlStatus::FEATURE_NOT_SUPPORTED
            }
            Self::InternerBudgetExceeded { .. } | Self::NestingLimitExceeded { .. } => {
                GqlStatus::PROGRAM_LIMIT_EXCEEDED
            }
        }
    }

    pub(crate) fn syntax(
        message: impl Into<String>,
        span: SourceSpan,
        hint: Option<String>,
    ) -> Self {
        Self::SyntaxError {
            status: GqlStatus::SYNTAX_ERROR,
            message: message.into(),
            span,
            hint,
        }
    }

    pub(crate) fn not_implemented(
        message: impl Into<String>,
        span: SourceSpan,
        hint: Option<&'static str>,
    ) -> Self {
        Self::NotImplemented {
            message: message.into(),
            span,
            hint: hint.map(str::to_owned),
        }
    }

    pub(crate) fn empty_program() -> Self {
        Self::syntax(
            "empty GQL program",
            SourceSpan::default(),
            Some("provide a RETURN statement".into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::GqlStatus;

    #[test]
    fn gqlstatus_codes_match_iso_table_8_remap() {
        let cases = [
            (GqlStatus::SYNTAX_ERROR, "42001", *b"42"),
            (GqlStatus::FEATURE_NOT_SUPPORTED, "42N01", *b"42"),
            (GqlStatus::PROGRAM_LIMIT_EXCEEDED, "5GQL1", *b"5G"),
            (GqlStatus::OPERATION_CANCELLED, "5GQL2", *b"5G"),
            (GqlStatus::DEADLINE_EXCEEDED, "5GQL3", *b"5G"),
            (GqlStatus::UNDEFINED_REFERENCE, "42N03", *b"42"),
            (GqlStatus::INVALID_REFERENCE, "42002", *b"42"),
            (GqlStatus::DUPLICATE_OBJECT, "42N10", *b"42"),
            (GqlStatus::DATATYPE_MISMATCH, "22G03", *b"22"),
            (GqlStatus::DATA_EXCEPTION, "22000", *b"22"),
            (GqlStatus::INVALID_TRANSACTION_STATE, "25000", *b"25"),
            (GqlStatus::INVALID_TRANSACTION_STATE_MIXING, "25G02", *b"25"),
            (GqlStatus::IN_FAILED_TRANSACTION, "25N02", *b"25"),
            (GqlStatus::UNKNOWN_PROCEDURE, "42N04", *b"42"),
            (GqlStatus::INVALID_PROCEDURE_ARGUMENT, "22G03", *b"22"),
            (GqlStatus::CAPABILITY_VIOLATION, "42N28", *b"42"),
            (GqlStatus::IMPLEMENTATION_DEFINED_ERROR, "5GQL0", *b"5G"),
        ];

        for (status, code, class) in cases {
            assert_eq!(status.as_str(), code);
            assert_eq!(status.class(), class);
        }
    }
}
