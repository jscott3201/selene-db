//! ISO Annex B implementation-defined identifier register.
//!
//! Split out of `feature_register.rs` to keep that file under the 700-LOC cap.
//! `AnnexBId` and `ImplDefinedChoice` live here and are re-exported from the
//! parent module so the public `feature_register::AnnexBId` path is preserved.

/// ISO Annex B implementation-defined identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct AnnexBId(&'static str);

impl AnnexBId {
    /// Return the ISO Annex B ID string, such as `IL001`.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Chosen value for an implementation-defined element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImplDefinedChoice {
    /// Human-readable summary of selene-db's choice.
    pub choice: &'static str,
    /// Spec section that owns the detailed behavior.
    pub settled_in: &'static str,
}

/// Canonical Annex B register entries settled by the current specs.
pub const ANNEX_B_REGISTER: &[(AnnexBId, ImplDefinedChoice)] = &[
    (
        AnnexBId("IA001"),
        ImplDefinedChoice {
            choice: "f64 default; Float32 distinct; NaN total_cmp",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IA025"),
        ImplDefinedChoice {
            choice: "numeric ordering follows total_cmp for f32/f64",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("ID001"),
        ImplDefinedChoice {
            choice: "caller-supplied principal bytes; opaque to selene-db",
            settled_in: "spec 04 section 3.2",
        },
    ),
    (
        AnnexBId("ID016"),
        ImplDefinedChoice {
            choice: "en-US diagnostic text by default",
            settled_in: "spec 09 section 5",
        },
    ),
    (
        AnnexBId("ID017"),
        ImplDefinedChoice {
            choice: "structured diagnostic map may carry selene provider fields",
            settled_in: "spec 06 section 3.3",
        },
    ),
    (
        AnnexBId("ID028"),
        ImplDefinedChoice {
            choice: "i64 default; i128 if context demands",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("ID034"),
        ImplDefinedChoice {
            choice: "28 significant digits via rust_decimal",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("ID037"),
        ImplDefinedChoice {
            choice: "binary64 default; binary32 if context demands",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("ID090"),
        ImplDefinedChoice {
            choice: "node terminology",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("ID091"),
        ImplDefinedChoice {
            choice: "edge terminology",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IE001"),
        ImplDefinedChoice {
            choice: "auto-commit per statement; explicit START TRANSACTION for multi-statement",
            settled_in: "spec 03 section 6.4",
        },
    ),
    (
        AnnexBId("IE002"),
        ImplDefinedChoice {
            choice: "serializable only",
            settled_in: "spec 03 section 6.4",
        },
    ),
    (
        AnnexBId("IE004"),
        ImplDefinedChoice {
            choice: "no relaxation from serializable",
            settled_in: "spec 03 section 6.4",
        },
    ),
    (
        AnnexBId("IE006"),
        ImplDefinedChoice {
            choice: "catalog statements inside data transactions are rejected",
            settled_in: "spec 03 section 6.4",
        },
    ),
    (
        AnnexBId("IE007"),
        ImplDefinedChoice {
            choice: "data mutations inside catalog transactions are rejected",
            settled_in: "spec 03 section 6.4",
        },
    ),
    (
        AnnexBId("IL001"),
        ImplDefinedChoice {
            choice: "node labels min 0; edge labels exactly 1",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IL013"),
        ImplDefinedChoice {
            choice: "2^32 - 1 bytes per inline string",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IL015"),
        ImplDefinedChoice {
            choice: "2^32 - 1 constructed-value elements",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IL018"),
        ImplDefinedChoice {
            choice: "path quantifier upper bound 100",
            settled_in: "spec 08 section 9",
        },
    ),
    (
        AnnexBId("IL020"),
        ImplDefinedChoice {
            choice: "flat catalog, nesting depth 0",
            settled_in: "spec 09 section 5",
        },
    ),
    (
        AnnexBId("IL024"),
        ImplDefinedChoice {
            choice: "nanosecond temporal precision",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IS001"),
        ImplDefinedChoice {
            choice: "caller-bounded session scope",
            settled_in: "spec 08 section 9",
        },
    ),
    (
        AnnexBId("IV001-IV016"),
        ImplDefinedChoice {
            choice: "Value enum closed substitution union",
            settled_in: "spec 02 section 3",
        },
    ),
    (
        AnnexBId("IV011"),
        ImplDefinedChoice {
            choice: "Value minus RecordTyped; Value::Vector is native dense f32; Value::Json is native RFC 8259 JSON; Value::Extended carries opaque sister-project payloads",
            settled_in: "spec 02 section 3.1",
        },
    ),
    (
        AnnexBId("IW001"),
        ImplDefinedChoice {
            choice: "caller responsibility per D1",
            settled_in: "spec 01 section 4",
        },
    ),
    (
        AnnexBId("IW002"),
        ImplDefinedChoice {
            choice: "caller responsibility per D1",
            settled_in: "spec 01 section 4",
        },
    ),
    (
        AnnexBId("IW007"),
        ImplDefinedChoice {
            choice: "raw code plus structured fields; miette for terminals",
            settled_in: "spec 09 section 6",
        },
    ),
    (
        AnnexBId("IW010"),
        ImplDefinedChoice {
            choice: "single frozen native BuiltinProcedureRegistry; no loadable extensions",
            settled_in: "spec 08",
        },
    ),
    (
        AnnexBId("IW014"),
        ImplDefinedChoice {
            choice: "byte-exact comparison only",
            settled_in: "spec 08 section 9",
        },
    ),
    (
        AnnexBId("IW015"),
        ImplDefinedChoice {
            choice: "no automatic directory/schema creation",
            settled_in: "spec 09 section 5",
        },
    ),
    (
        AnnexBId("IW016"),
        ImplDefinedChoice {
            choice: "no automatic directory/schema creation",
            settled_in: "spec 09 section 5",
        },
    ),
    (
        AnnexBId("IW025"),
        ImplDefinedChoice {
            choice: "catalog-modifying procedures are transactional via Mutator::commit",
            settled_in: "spec 09 section 5",
        },
    ),
    (
        AnnexBId("ID048"),
        ImplDefinedChoice {
            choice: "default session time zone displacement is UTC (+00:00); SESSION RESET TIME ZONE restores it",
            settled_in: "ISO section 4.5.2.2 / section 7.2 GR3",
        },
    ),
    (
        AnnexBId("ID049"),
        ImplDefinedChoice {
            choice: "default session parameters are the empty set; no impl-defined `_`-prefixed parameters are pre-bound",
            settled_in: "ISO section 4.5.2.2 / section 7.2 GR4-5",
        },
    ),
];
