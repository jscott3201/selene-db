//! GQLSTATUS codes emitted by selene-db plus symbolic names.

/// `(code, human-readable name)` pairs for GQLSTATUS values selene-db emits.
///
/// Standard codes use their ISO/IEC 39075:2024 section 23.1 Table 8
/// condition names where available. `0Gxxx`, `42Nxx`, `25N02`, and `5GQL1`
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
    ("22000", "data-exception"),
    ("22003", "numeric-value-out-of-range"),
    // kept for selene-core, selene-graph, and selene-persist compatibility - TODO migrate
    // in a cross-crate GQLSTATUS audit.
    ("22023", "data-exception-invalid-parameter-value"),
    ("22G03", "invalid-value-type"),
    ("25G02", "invalid-transaction-state-mixing"),
    ("25N02", "in-failed-transaction"),
    ("42001", "invalid-syntax"),
    ("42002", "invalid-reference"),
    ("42N01", "feature-not-supported"),
    ("42N03", "undefined-reference"),
    ("42N04", "unknown-procedure"),
    ("42N10", "duplicate-object"),
    ("42N28", "capability-violation"),
    ("53000", "insufficient-resources"),
    // kept for selene-core and selene-persist compatibility - TODO migrate in a
    // cross-crate GQLSTATUS audit.
    ("54000", "program-limit-exceeded"),
    ("5GQL1", "program-limit-exceeded"),
    ("XX500", "implementation-defined-error"),
    ("XX501", "graph-mutation-error"),
    ("XX502", "durability-flush-error"),
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
