//! GQLSTATUS codes emitted by selene-db plus symbolic names.

/// `(code, human-readable name)` pairs for GQLSTATUS values selene-db emits.
///
/// Standard codes use their ISO/IEC 39075:2024 section 23.1 Table 8
/// condition names where available. `0Gxxx`, `42Nxx`, `25N02`, and `5GQLx`
/// rows are selene-db implementation-defined subclasses or classes per
/// section 23.1's implementation-defined ranges.
pub const ALL_GQLSTATUS_NAMES: &[(&str, &str)] = &[
    ("00000", "successful-completion"),
    ("08000", "connection-exception"),
    ("0G001", "extension-type-id-conflict"),
    ("0G002", "extension-type-id-unregistered"),
    ("0G003", "zero-identifier"),
    ("0G004", "transient-codec-error"),
    ("0G008", "compact-key-value-length-mismatch"),
    ("0G009", "overlapping-diff"),
    ("01G11", "null-value-eliminated-in-set-function"),
    ("01N01", "validation-mode-relaxed-write"),
    ("22000", "data-exception"),
    ("22003", "numeric-value-out-of-range"),
    ("22011", "substring-error"),
    ("22012", "division-by-zero"),
    ("22018", "invalid-character-value-for-cast"),
    ("2201E", "invalid-argument-for-natural-logarithm"),
    ("2201F", "invalid-argument-for-power-function"),
    ("22027", "trim-error"),
    ("22G03", "invalid-value-type"),
    ("22G04", "values-not-comparable"),
    ("22G0C", "list-element-error"),
    ("22G0M", "multiple-assignments-to-graph-element-property"),
    (
        "22G0S",
        "number-of-node-properties-exceeds-supported-maximum",
    ),
    (
        "22G0T",
        "number-of-edge-properties-exceeds-supported-maximum",
    ),
    ("22G0U", "record-fields-do-not-match"),
    ("22G0X", "record-data-field-unassignable"),
    ("25G01", "active-gql-transaction"),
    ("25G02", "invalid-transaction-state-mixing"),
    ("25G03", "read-only-gql-transaction"),
    ("25N02", "in-failed-transaction"),
    ("2D000", "invalid-transaction-termination"),
    ("42001", "invalid-syntax"),
    ("42002", "invalid-reference"),
    ("42N01", "feature-not-supported"),
    ("42N03", "undefined-reference"),
    ("42N04", "unknown-procedure"),
    ("42N10", "duplicate-object"),
    ("42N28", "capability-violation"),
    ("53000", "insufficient-resources"),
    ("5GQL1", "program-limit-exceeded"),
    ("5GQL2", "operation-cancelled"),
    ("5GQL3", "deadline-exceeded"),
    ("5GQL0", "implementation-defined-error"),
    ("G1001", "dependent-object-still-exists"),
    ("G2000", "graph-type-violation"),
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
    fn gqlstatus_2201e_round_trip() {
        assert_eq!(
            gqlstatus_name("2201E"),
            Some("invalid-argument-for-natural-logarithm")
        );
    }

    #[test]
    fn gqlstatus_22011_round_trip() {
        assert_eq!(gqlstatus_name("22011"), Some("substring-error"));
    }

    #[test]
    fn gqlstatus_22027_round_trip() {
        assert_eq!(gqlstatus_name("22027"), Some("trim-error"));
    }

    #[test]
    fn gqlstatus_name_unknown_code_returns_none() {
        assert_eq!(gqlstatus_name("99999"), None);
    }
}
