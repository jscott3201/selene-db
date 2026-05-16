//! Core error types and ISO GQLSTATUS mappings.

use crate::extension_type_ids::ExtensionTypeId;
use crate::istr::IStr;

/// Result alias for `selene-core` operations.
pub type CoreResult<T> = Result<T, CoreError>;

/// Error type for foundation data-model operations.
///
/// Codes in the `0Gxxx` range are selene-db implementation-defined conditions
/// reserved for engine-internal validation and registry failures.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum CoreError {
    /// The process-global string interner reached its distinct-string cap.
    #[error("interner cap exceeded: {count} distinct strings (max {max})")]
    #[diagnostic(code(SLENE_C_001), help("see Spec 02 §5.1"))]
    IStrCapExceeded {
        /// Number of distinct strings currently interned.
        count: usize,
        /// Maximum allowed distinct interned strings.
        max: usize,
    },

    /// A string or byte-string exceeded the implementation-defined length.
    #[error("string too long: {got} bytes (max {max})")]
    #[diagnostic(code(SLENE_C_002))]
    StringTooLong {
        /// Observed byte length.
        got: usize,
        /// Maximum byte length.
        max: u32,
    },

    /// A list or record exceeded the implementation-defined cardinality.
    #[error("constructed value too large: {got} elements (max {max})")]
    #[diagnostic(code(SLENE_C_003))]
    ConstructedValueTooLarge {
        /// Observed element count.
        got: usize,
        /// Maximum element count.
        max: u32,
    },

    /// A decimal exceeded the v1.0 significant-digit precision.
    #[error("decimal precision exceeded: {got} significant digits (max {max})")]
    #[diagnostic(code(SLENE_C_004))]
    DecimalPrecisionExceeded {
        /// Observed significant-digit count.
        got: u32,
        /// Maximum significant-digit count.
        max: u32,
    },

    /// An extension type ID was registered by more than one adapter.
    #[error("extension type id conflict: {type_id:?} already registered")]
    #[diagnostic(code(SLENE_C_005))]
    ExtensionTypeIdConflict {
        /// Conflicting extension type ID.
        type_id: ExtensionTypeId,
    },

    /// No adapter is registered for the requested extension type ID.
    #[error("extension type id unregistered: {type_id:?}")]
    #[diagnostic(code(SLENE_C_006))]
    ExtensionTypeIdUnregistered {
        /// Missing extension type ID.
        type_id: ExtensionTypeId,
    },

    /// Identifier value zero is reserved as the tombstone sentinel.
    #[error("invalid identifier: zero is reserved as tombstone sentinel")]
    #[diagnostic(code(SLENE_C_007))]
    ZeroIdentifier,

    /// Compact `PropertyMap` was constructed with mismatched key and value counts.
    #[error("compact property map key/value length mismatch: {keys} keys, {values} values")]
    #[diagnostic(code(SLENE_C_008))]
    CompactKeyValueLengthMismatch {
        /// Number of keys supplied.
        keys: usize,
        /// Number of value slots supplied.
        values: usize,
    },

    /// A label diff or property diff named the same key in both add/set and remove.
    #[error("overlapping {kind} diff: key {key} appears in both add/set and remove")]
    #[diagnostic(code(SLENE_C_009))]
    OverlappingDiff {
        /// `"label"` or `"property"`.
        kind: &'static str,
        /// The contradicting key.
        key: IStr,
    },
}

impl CoreError {
    /// Map this error to its 5-character ISO GQLSTATUS code.
    ///
    /// ISO/IEC 39075:2024 clause 23 defines the status-code shape. Spec 02
    /// section 3.1 binds the value-limit and numeric-limit choices used here.
    #[must_use]
    pub const fn gqlstatus(&self) -> &'static str {
        match self {
            Self::IStrCapExceeded { .. } => "54000",
            Self::StringTooLong { .. } | Self::ConstructedValueTooLarge { .. } => "22023",
            Self::DecimalPrecisionExceeded { .. } => "22003",
            Self::ExtensionTypeIdConflict { .. } => "0G001",
            Self::ExtensionTypeIdUnregistered { .. } => "0G002",
            Self::ZeroIdentifier => "0G003",
            Self::CompactKeyValueLengthMismatch { .. } => "0G008",
            Self::OverlappingDiff { .. } => "0G009",
        }
    }
}

#[cfg(test)]
mod tests {
    use miette::Diagnostic;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(CoreError::IStrCapExceeded { count: 2, max: 1 }, "54000", "SLENE_C_001")]
    #[case(CoreError::StringTooLong { got: 2, max: 1 }, "22023", "SLENE_C_002")]
    #[case(
        CoreError::ConstructedValueTooLarge { got: 2, max: 1 },
        "22023",
        "SLENE_C_003"
    )]
    #[case(
        CoreError::DecimalPrecisionExceeded { got: 29, max: 28 },
        "22003",
        "SLENE_C_004"
    )]
    #[case(
        CoreError::ExtensionTypeIdConflict { type_id: ExtensionTypeId(0x100) },
        "0G001",
        "SLENE_C_005"
    )]
    #[case(
        CoreError::ExtensionTypeIdUnregistered { type_id: ExtensionTypeId(0x100) },
        "0G002",
        "SLENE_C_006"
    )]
    #[case(CoreError::ZeroIdentifier, "0G003", "SLENE_C_007")]
    #[case(
        CoreError::CompactKeyValueLengthMismatch { keys: 2, values: 1 },
        "0G008",
        "SLENE_C_008"
    )]
    #[case(
        CoreError::OverlappingDiff { kind: "label", key: crate::intern("err.test.overlap").unwrap() },
        "0G009",
        "SLENE_C_009"
    )]
    fn gqlstatus_and_diagnostic_code_match(
        #[case] error: CoreError,
        #[case] gqlstatus: &str,
        #[case] diagnostic_code: &str,
    ) {
        assert_eq!(error.gqlstatus(), gqlstatus);
        assert!(
            crate::gqlstatus_name(gqlstatus).is_some(),
            "GQLSTATUS code {gqlstatus} emitted by CoreError but not in ALL_GQLSTATUS_NAMES"
        );
        assert_eq!(
            error.code().map(|code| code.to_string()).as_deref(),
            Some(diagnostic_code)
        );
    }

    #[test]
    fn display_includes_structured_field_values() {
        let error = CoreError::IStrCapExceeded { count: 7, max: 3 };
        let rendered = error.to_string();
        assert!(rendered.contains('7'));
        assert!(rendered.contains('3'));
    }
}
