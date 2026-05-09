//! GQLSTATUS codes emitted by selene-db plus symbolic names.

/// `(code, human-readable name)` pairs for GQLSTATUS values selene-db emits.
///
/// Standard codes use their ISO/SQL condition names where available. `0Gxxx`
/// rows are selene-core implementation-defined slugs matching the emitting
/// error variant because ISO/IEC 39075 does not assign these local subclasses.
pub const ALL_GQLSTATUS_NAMES: &[(&str, &str)] = &[
    ("00000", "successful-completion"),
    ("08000", "connection-exception"),
    ("0A000", "feature-not-supported"),
    ("0G001", "extension-type-id-conflict"),
    ("0G002", "extension-type-id-unregistered"),
    ("0G003", "zero-identifier"),
    ("0G004", "transient-codec-error"),
    ("0G008", "compact-key-value-length-mismatch"),
    ("0G009", "overlapping-diff"),
    ("22000", "data-exception"),
    ("22003", "numeric-value-out-of-range"),
    ("22023", "data-exception-invalid-parameter-value"),
    ("25G02", "invalid-transaction-state-mixing"),
    ("42002", "invalid-reference"),
    ("42601", "syntax-error-or-access-rule-violation"),
    ("42703", "undefined-reference"),
    ("42710", "duplicate-object"),
    ("42883", "datatype-mismatch"),
    ("53000", "insufficient-resources"),
    ("54000", "program-limit-exceeded"),
    ("XX500", "implementation-defined-error"),
];

/// Return the human-readable name for a GQLSTATUS code.
#[must_use]
pub fn gqlstatus_name(code: &str) -> Option<&'static str> {
    ALL_GQLSTATUS_NAMES
        .iter()
        .find_map(|(candidate, name)| (*candidate == code).then_some(*name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gqlstatus_name_known_codes_round_trip() {
        assert!(ALL_GQLSTATUS_NAMES.len() >= 5);
        for &(code, name) in ALL_GQLSTATUS_NAMES {
            assert_eq!(gqlstatus_name(code), Some(name));
        }
    }

    #[test]
    fn gqlstatus_name_unknown_code_returns_none() {
        assert_eq!(gqlstatus_name("99999"), None);
    }
}
