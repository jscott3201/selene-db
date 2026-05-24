//! ISO/IEC 39075:2024 feature and implementation-defined registers.
//!
//! This file is the canonical source for selene-db's v1.0 language claim.
//! The markdown tables for Spec 01, Spec 07, and Spec 09 are rendered or
//! checked from these constants by `build/regen_feature_docs.sh`.

use std::fmt;

/// Stable ISO GQL feature identifier.
///
/// The private field makes the set closed to this module while preserving the
/// spec's string IDs as the stable ABI-facing representation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct FeatureId(&'static str);

impl FeatureId {
    /// Return the ISO feature ID string, such as `GP04`.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for FeatureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! feature_ids {
    ($($name:ident = $id:literal => $display:literal;)*) => {
        impl FeatureId {
            $(
                #[doc = concat!("`", $display, "`")]
                pub const $name: FeatureId = FeatureId($id);
            )*
        }

        /// Every feature ID currently referenced by selene-db specs.
        pub const REFERENCED_FEATURES: &[(FeatureId, &str)] = &[
            $((FeatureId::$name, $display),)*
        ];
    };
}

feature_ids! {
    G002 = "G002" => "Different edges match mode";
    G003 = "G003" => "Repeatable elements match mode";
    G010 = "G010" => "Explicit WALK keyword";
    G011 = "G011" => "Advanced path modes: TRAIL";
    G012 = "G012" => "Advanced path modes: SIMPLE";
    G013 = "G013" => "Advanced path modes: ACYCLIC";
    G015 = "G015" => "All path search: explicit ALL keyword";
    G016 = "G016" => "Any path search";
    G017 = "G017" => "All shortest path search";
    G018 = "G018" => "Any shortest path search";
    G019 = "G019" => "Counted shortest path search";
    G020 = "G020" => "Counted shortest group search";
    G036 = "G036" => "Quantified edge pattern";
    G037 = "G037" => "Questioned path primary";
    G060 = "G060" => "Bounded quantified path primary";
    G061 = "G061" => "Unbounded quantified path primary";
    G110 = "G110" => "IS DIRECTED predicate";
    G111 = "G111" => "IS LABELED predicate";
    G112 = "G112" => "IS SOURCE and IS DESTINATION predicate";
    G113 = "G113" => "ALL_DIFFERENT predicate";
    G114 = "G114" => "SAME predicate";
    G115 = "G115" => "PROPERTY_EXISTS predicate";
    GA01 = "GA01" => "IEEE 754 floating point operations";
    GA07 = "GA07" => "Ordering by discarded binding variables";
    GC02 = "GC02" => "Graph schema management: IF [ NOT ] EXISTS";
    GC03 = "GC03" => "Graph type: IF [ NOT ] EXISTS";
    GC04 = "GC04" => "Graph management";
    GC05 = "GC05" => "Graph management: IF [ NOT ] EXISTS";
    GD01 = "GD01" => "Updatable graphs";
    GE04 = "GE04" => "Parameters";
    GE05 = "GE05" => "Named parameters";
    GE07 = "GE07" => "XOR operator";
    GE08 = "GE08" => "CAST operator";
    GF13 = "GF13" => "SIZE function";
    GH02 = "GH02" => "Undirected edge patterns";
    GG01 = "GG01" => "Graph with an open graph type";
    GG02 = "GG02" => "Graph with a closed graph type";
    GG20 = "GG20" => "Explicit element type names";
    GG21 = "GG21" => "Explicit element type key label sets";
    GP01 = "GP01" => "Inline procedures";
    GP02 = "GP02" => "Inline procedures: simple";
    GP03 = "GP03" => "Inline procedures: nested";
    GP04 = "GP04" => "Named procedure calls";
    GP05 = "GP05" => "Procedure-local value variable definitions";
    GP06 = "GP06" => "Procedure-local value variables based on simple expressions";
    GP07 = "GP07" => "Procedure-local value variable based on subqueries";
    GP08 = "GP08" => "Procedure-local binding table variable definitions";
    GP09 = "GP09" => "Procedure-local binding table variables based on simple expressions";
    GP10 = "GP10" => "Procedure-local binding table variables based on query expressions";
    GP11 = "GP11" => "Procedure-local graph variable definitions";
    GP12 = "GP12" => "Procedure-local graph variables based on simple graph expressions";
    GP13 = "GP13" => "Procedure-local graph variables based on subqueries";
    GP14 = "GP14" => "Binding tables as procedure arguments";
    GP15 = "GP15" => "Graphs as procedure arguments";
    GP18 = "GP18" => "Mixed catalog/data transaction feature";
    GQ03 = "GQ03" => "Composite query: UNION";
    GQ04 = "GQ04" => "Composite query: EXCEPT DISTINCT";
    GQ05 = "GQ05" => "Composite query: EXCEPT ALL";
    GQ06 = "GQ06" => "Composite query: INTERSECT DISTINCT";
    GQ07 = "GQ07" => "Composite query: INTERSECT ALL";
    GQ08 = "GQ08" => "FILTER statement";
    GQ09 = "GQ09" => "Composite query: OTHERWISE";
    GQ12 = "GQ12" => "OFFSET clause";
    GQ13 = "GQ13" => "LIMIT clause";
    GQ15 = "GQ15" => "GROUP BY clause";
    GQ18 = "GQ18" => "Scalar value query expression";
    GQ20 = "GQ20" => "Linear query composition";
    GT01 = "GT01" => "Explicit transaction commands";
    GT03 = "GT03" => "Multi-graph transactions";
    GV01 = "GV01" => "8 bit unsigned integer numbers";
    GV02 = "GV02" => "8 bit signed integer numbers";
    GV03 = "GV03" => "16 bit unsigned integer numbers";
    GV04 = "GV04" => "16 bit signed integer numbers";
    GV05 = "GV05" => "Small unsigned integer numbers";
    GV06 = "GV06" => "32 bit unsigned integer numbers";
    GV07 = "GV07" => "32 bit signed integer numbers";
    GV08 = "GV08" => "Regular unsigned integer numbers";
    GV09 = "GV09" => "Specified integer number precision";
    GV10 = "GV10" => "Big unsigned integer numbers";
    GV11 = "GV11" => "64 bit unsigned integer numbers";
    GV12 = "GV12" => "64 bit signed integer numbers";
    GV13 = "GV13" => "128 bit unsigned integer numbers";
    GV14 = "GV14" => "128 bit signed integer numbers";
    GV15 = "GV15" => "256 bit unsigned integer numbers";
    GV16 = "GV16" => "256 bit signed integer numbers";
    GV17 = "GV17" => "Decimal numbers";
    GV18 = "GV18" => "Small signed integer numbers";
    GV19 = "GV19" => "Big signed integer numbers";
    GV20 = "GV20" => "16 bit floating point numbers";
    GV21 = "GV21" => "32 bit floating point numbers";
    GV22 = "GV22" => "Specified floating point number precision";
    GV23 = "GV23" => "Floating point type name synonyms";
    GV24 = "GV24" => "64 bit floating point numbers";
    GV25 = "GV25" => "128 bit floating point numbers";
    GV26 = "GV26" => "256 bit floating point numbers";
    GV35 = "GV35" => "Byte string types";
    GV39 = "GV39" => "Temporal types: date, local datetime and local time support";
    GV40 = "GV40" => "Temporal types: zoned datetime and zoned time support";
    GV41 = "GV41" => "Temporal types: duration support";
    GV45 = "GV45" => "Record types";
    GV46 = "GV46" => "Closed record types";
    GV47 = "GV47" => "Open record types";
    GV48 = "GV48" => "Nested record types";
    GV50 = "GV50" => "List value types";
    GV55 = "GV55" => "Path value types";
    GV60 = "GV60" => "Graph reference value types";
    GV61 = "GV61" => "Binding table reference value types";
    GV90 = "GV90" => "Explicit value type nullability";
}

/// v1.0 supported optional feature set.
///
/// ISO sources: Annex A numbered pp. 522-554; Annex D Table D.1 numbered
/// pp. 577-586. Implication closure is handled by the flagger/planner.
pub const SUPPORTED_FEATURES: &[FeatureId] = &[
    FeatureId::G010,
    FeatureId::G011,
    FeatureId::G012,
    FeatureId::G013,
    FeatureId::G015,
    FeatureId::G016,
    FeatureId::G017,
    FeatureId::G018,
    FeatureId::G036,
    FeatureId::G037,
    FeatureId::G060,
    FeatureId::G061,
    FeatureId::G110,
    FeatureId::G111,
    FeatureId::G112,
    FeatureId::G113,
    FeatureId::G114,
    FeatureId::G115,
    FeatureId::GA01,
    FeatureId::GA07,
    FeatureId::GC03,
    FeatureId::GD01,
    FeatureId::GE04,
    FeatureId::GE05,
    FeatureId::GE07,
    FeatureId::GE08,
    FeatureId::GF13,
    FeatureId::GH02,
    FeatureId::GG01,
    FeatureId::GG02,
    FeatureId::GG20,
    FeatureId::GG21,
    FeatureId::GP01,
    FeatureId::GP02,
    FeatureId::GP04,
    FeatureId::GQ03,
    FeatureId::GQ04,
    FeatureId::GQ05,
    FeatureId::GQ06,
    FeatureId::GQ07,
    FeatureId::GQ08,
    FeatureId::GQ09,
    FeatureId::GQ12,
    FeatureId::GQ13,
    FeatureId::GQ15,
    FeatureId::GQ18,
    FeatureId::GQ20,
    FeatureId::GT01,
    FeatureId::GV01,
    FeatureId::GV02,
    FeatureId::GV03,
    FeatureId::GV04,
    FeatureId::GV05,
    FeatureId::GV06,
    FeatureId::GV07,
    FeatureId::GV08,
    FeatureId::GV09,
    FeatureId::GV10,
    FeatureId::GV11,
    FeatureId::GV12,
    FeatureId::GV13,
    FeatureId::GV14,
    FeatureId::GV17,
    FeatureId::GV18,
    FeatureId::GV19,
    FeatureId::GV21,
    FeatureId::GV24,
    FeatureId::GV35,
    FeatureId::GV39,
    FeatureId::GV40,
    FeatureId::GV41,
    FeatureId::GV50,
    FeatureId::GV55,
];

/// Rationale for referenced optional features not claimed in v1.0.
pub const NOT_SUPPORTED_RATIONALE: &[(FeatureId, &str)] = &[
    (
        FeatureId::G002,
        "DIFFERENT EDGES match mode is a graph-pattern-wide traversal policy deferred from v1.1",
    ),
    (
        FeatureId::G003,
        "REPEATABLE ELEMENTS match mode is a graph-pattern-wide traversal policy deferred from v1.1",
    ),
    (
        FeatureId::G019,
        "counted shortest selectors require grammar support and counted-path selector semantics",
    ),
    (
        FeatureId::G020,
        "counted shortest selectors require grammar support and counted-path selector semantics",
    ),
    (
        FeatureId::GP03,
        "explicit variable-scope inline procedures are deferred from v1.1",
    ),
    (
        FeatureId::GP05,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP06,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP07,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP08,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP09,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP10,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP11,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP12,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP13,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP14,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP15,
        "procedure-local definitions require the procedure body parser; unsupported in v1.0",
    ),
    (
        FeatureId::GP18,
        "mixed catalog/data transaction behavior remains forbidden in v1.0",
    ),
    (
        FeatureId::GC02,
        "CREATE/DROP SCHEMA is outside the v1.0 catalog claim (graph-schema vs graph-type vs graph)",
    ),
    (
        FeatureId::GC04,
        "CREATE/DROP GRAPH parses but graph management DDL remains outside the v1.0 catalog claim",
    ),
    (
        FeatureId::GC05,
        "graph management IF [NOT] EXISTS modifiers remain outside the v1.0 catalog claim",
    ),
    (
        FeatureId::GT03,
        "multi-graph transactions are out of v1.0 scope",
    ),
    (
        FeatureId::GV15,
        "256-bit unsigned integers are not represented in Value v1",
    ),
    (
        FeatureId::GV16,
        "256-bit signed integers are not represented in Value v1",
    ),
    (
        FeatureId::GV20,
        "REAL spelling is outside the v1.0 claim; FLOAT16 remains deferred",
    ),
    (
        FeatureId::GV22,
        "specified floating precision syntax is deferred",
    ),
    (
        FeatureId::GV23,
        "REAL/DOUBLE synonyms are deferred until parser coverage is explicit",
    ),
    (FeatureId::GV25, "FLOAT128 is deferred"),
    (FeatureId::GV26, "FLOAT256 is deferred"),
    (
        FeatureId::GV45,
        "record type expressions require type_name grammar + RecordType builder; reclaim with the type-system extension brief",
    ),
    (
        FeatureId::GV46,
        "record type expressions require type_name grammar + RecordType builder; reclaim with the type-system extension brief",
    ),
    (
        FeatureId::GV47,
        "record type expressions require type_name grammar + RecordType builder; reclaim with the type-system extension brief",
    ),
    (
        FeatureId::GV48,
        "record type expressions require type_name grammar + RecordType builder; reclaim with the type-system extension brief",
    ),
    (
        FeatureId::GV60,
        "GRAPH/TABLE reference type spellings require type_name grammar + reference-type builder; reclaim alongside record types",
    ),
    (
        FeatureId::GV61,
        "GRAPH/TABLE reference type spellings require type_name grammar + reference-type builder; reclaim alongside record types",
    ),
    (
        FeatureId::GV90,
        "explicit value type nullability requires type-level nullability on GqlType; reclaim once the type AST carries the marker",
    ),
];

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
            choice: "serializable only in v1.0",
            settled_in: "spec 03 section 6.4",
        },
    ),
    (
        AnnexBId("IE004"),
        ImplDefinedChoice {
            choice: "no relaxation from serializable in v1.0",
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
            choice: "Value minus RecordTyped; registered Extended values allowed",
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
            choice: "procedure-pack model",
            settled_in: "spec 05",
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
];

/// True when `id` is in the v1.0 supported feature set.
pub fn is_supported(id: FeatureId) -> bool {
    SUPPORTED_FEATURES.contains(&id)
}

/// Return the ISO display name for a referenced feature ID.
pub fn name_of(id: FeatureId) -> Option<&'static str> {
    REFERENCED_FEATURES
        .iter()
        .find_map(|(feature, name)| (*feature == id).then_some(*name))
}

/// Return a referenced feature ID from its stable string representation.
pub fn feature_id_from_str(id: &str) -> Option<FeatureId> {
    REFERENCED_FEATURES
        .iter()
        .find_map(|(feature, _)| (feature.as_str() == id).then_some(*feature))
}

/// Return the v1.0 non-support rationale for a referenced feature ID.
pub fn non_supported_rationale(id: FeatureId) -> Option<&'static str> {
    NOT_SUPPORTED_RATIONALE
        .iter()
        .find_map(|(feature, rationale)| (*feature == id).then_some(*rationale))
}
