//! ISO/IEC 39075:2024 feature and implementation-defined registers.
//!
//! This file is the canonical source for selene-db's optional-feature language
//! claim. The markdown tables for Spec 01, Spec 07, and Spec 09 are rendered or
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
    G014 = "G014" => "Explicit PATH/PATHS keywords";
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
    G100 = "G100" => "ELEMENT_ID function";
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
    // ISO/IEC 39075:2024 Annex D Table D.1 row 77 / subclause 17.7: GE08 is
    // "Reference parameters", NOT a CAST feature. selene-db does not implement
    // reference parameters, so GE08 is referenced-but-not-supported (see
    // NOT_SUPPORTED_RATIONALE). CAST is `<cast specification>` (§20.8 /
    // Table D.1 row 53 = GA05, "Cast specification"). Per ISO Annex A item 52
    // (Feature GA05), without GA05 "conforming GQL language shall not contain a
    // <cast specification>" — CAST is gated behind GA05, NOT baseline. selene-db
    // implements the cast construct, so it claims GA05 in SUPPORTED_FEATURES and
    // stamps it on every `ValueExpr::Cast`.
    GE08 = "GE08" => "Reference parameters";
    GA05 = "GA05" => "Cast specification";
    GF01 = "GF01" => "Enhanced numeric functions";
    GF02 = "GF02" => "Trigonometric functions";
    GF03 = "GF03" => "Logarithmic functions";
    GF05 = "GF05" => "Multi-character TRIM function";
    GF06 = "GF06" => "Explicit TRIM function";
    GF10 = "GF10" => "Advanced aggregate functions: general set functions";
    GF11 = "GF11" => "Advanced aggregate functions: binary set functions";
    GF12 = "GF12" => "CARDINALITY function";
    GF13 = "GF13" => "SIZE function";
    IM_UUID = "IM_UUID" => "selene-db UUID extension";
    IM_EXTENDS = "IM_EXTENDS" => "selene-db EXTENDS type composition extension";
    IM_INDEX_DDL = "IM_INDEX_DDL" => "selene-db named index DDL extension";
    IM_TYPED_PARAMS = "IM_TYPED_PARAMS" => "selene-db inline typed parameter declaration extension";
    IM_TRUNCATE = "IM_TRUNCATE" => "selene-db bulk truncate extension";
    IM_DROP_CASCADE = "IM_DROP_CASCADE" => "selene-db cascading DROP TYPE extension";
    IM_LIST_SUBSCRIPT = "IM_LIST_SUBSCRIPT" => "selene-db 1-based list element subscript extension";
    IM_DROP_GRAPH = "IM_DROP_GRAPH" => "selene-db DROP GRAPH factory-reset extension";
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
    GS01 = "GS01" => "SESSION SET command: session-local graph parameters";
    GS02 = "GS02" => "SESSION SET command: session-local binding table parameters";
    GS03 = "GS03" => "SESSION SET command: session-local value parameters";
    GS04 = "GS04" => "SESSION RESET command: reset all characteristics";
    GS05 = "GS05" => "SESSION RESET command: reset session schema";
    GS06 = "GS06" => "SESSION RESET command: reset session graph";
    GS07 = "GS07" => "SESSION RESET command: reset time zone displacement";
    GS08 = "GS08" => "SESSION RESET command: reset all session parameters";
    GS10 = "GS10" => "Session-local binding table parameters based on subqueries";
    GS11 = "GS11" => "Session-local value parameters based on subqueries";
    GS12 = "GS12" => "Session-local graph parameters based on simple graph expressions or references";
    GS13 = "GS13" => "Session-local binding table parameters based on simple expressions or references";
    GS14 = "GS14" => "Session-local value parameters based on simple expressions";
    GS15 = "GS15" => "SESSION SET command: set time zone displacement";
    GS16 = "GS16" => "SESSION RESET command: reset individual session parameters";
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

/// Supported optional feature set.
///
/// ISO sources: Annex A numbered pp. 522-554; Annex D Table D.1 numbered
/// pp. 577-586. Implication closure is handled by the flagger/planner.
pub const SUPPORTED_FEATURES: &[FeatureId] = &[
    FeatureId::G002,
    FeatureId::G003,
    FeatureId::G010,
    FeatureId::G011,
    FeatureId::G012,
    FeatureId::G013,
    // G014 "Explicit PATH/PATHS keywords" (ISO §16.6 <path or paths>, Annex A
    // §16.6 CR5). The match-prefix grammar parses the optional PATH/PATHS sugar
    // and the flagger stamps G014 iff it is present. Pure surface sugar per
    // §1.2.4 — inert at runtime. G014 has NO ISO §24.7 implied-feature
    // relationship (it appears in neither column of Table 10).
    FeatureId::G014,
    FeatureId::G015,
    FeatureId::G016,
    FeatureId::G017,
    FeatureId::G018,
    FeatureId::G019,
    FeatureId::G020,
    FeatureId::G036,
    FeatureId::G037,
    FeatureId::G060,
    FeatureId::G061,
    FeatureId::G100,
    FeatureId::G110,
    FeatureId::G111,
    FeatureId::G112,
    FeatureId::G113,
    FeatureId::G114,
    FeatureId::G115,
    FeatureId::GA01,
    FeatureId::GA05,
    FeatureId::GA07,
    FeatureId::GC03,
    FeatureId::GD01,
    FeatureId::GE04,
    FeatureId::GE05,
    FeatureId::GE07,
    FeatureId::GF01,
    FeatureId::GF02,
    FeatureId::GF03,
    FeatureId::GF05,
    FeatureId::GF06,
    FeatureId::GF10,
    FeatureId::GF11,
    FeatureId::GF12,
    FeatureId::GF13,
    FeatureId::IM_UUID,
    FeatureId::IM_EXTENDS,
    FeatureId::IM_INDEX_DDL,
    FeatureId::IM_TYPED_PARAMS,
    FeatureId::IM_TRUNCATE,
    FeatureId::IM_DROP_CASCADE,
    FeatureId::IM_LIST_SUBSCRIPT,
    FeatureId::IM_DROP_GRAPH,
    FeatureId::GH02,
    FeatureId::GG01,
    FeatureId::GG02,
    FeatureId::GG20,
    // GG21 "Explicit element type key label sets" — the type-DDL grammar now
    // parses the explicit `<...type key label set>` (`[ <label set phrase> ]
    // <implies>`, the `=>` marker, ISO §18.2/18.3) and the flagger stamps it.
    // Per the §24.7 implied-feature-relationships table GG21 implies GG02
    // ("Graph with a closed graph type"), which is already claimed above, so
    // the claim is implication-consistent. selene-db sets the IL003 key-label-
    // set cardinality cap to 1 (singleton), rejecting cardinality 0 (42012/
    // 42014) and > 1 (42013/42015) per §18.2 SR10/SR11 + §18.3 SR11/SR12 — a
    // conforming impl-defined cap, not a missing feature.
    FeatureId::GG21,
    FeatureId::GP01,
    FeatureId::GP02,
    FeatureId::GP03,
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
    FeatureId::GS03,
    FeatureId::GS04,
    FeatureId::GS07,
    FeatureId::GS08,
    FeatureId::GS15,
    FeatureId::GS16,
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
    FeatureId::GV45,
    FeatureId::GV46,
    FeatureId::GV47,
    FeatureId::GV48,
    FeatureId::GV50,
    FeatureId::GV55,
];

/// Rationale for referenced optional features not currently claimed.
///
/// This list is reserved for features that are *parser-reachable but rejected*:
/// each entry is backed by a negative conformance-corpus case proving the
/// `UnsupportedFeature` rejection (see `corpus_covers_feature_register`).
/// Features that have no syntactic surface at all — and so can be neither
/// claimed nor rejected — are NOT listed here; they remain in
/// `REFERENCED_FEATURES` only and surface as the `"referenced"` status in
/// `selene.feature_status()`. GE08 ("Reference parameters", §17.7 —
/// unimplemented) is deliberately in that referenced-only bucket
/// (CONFORMANCE-00). GA05 ("Cast specification", §20.8) is CLAIMED in
/// `SUPPORTED_FEATURES`: CAST is gated behind GA05 per ISO Annex A item 52 and
/// selene-db implements the cast construct. GG21 ("Explicit element type key
/// label sets", §18.2/18.3) is now CLAIMED as well: the type-DDL grammar parses
/// the explicit `<...type key label set>` (`=>` marker) and bounds its
/// cardinality to the IL003 singleton cap.
pub const NOT_SUPPORTED_RATIONALE: &[(FeatureId, &str)] = &[
    (
        FeatureId::GP05,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP06,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP07,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP08,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP09,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP10,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP11,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP12,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP13,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP14,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP15,
        "procedure-local definitions require the procedure body parser; not yet supported",
    ),
    (
        FeatureId::GP18,
        "mixed catalog/data transaction behavior remains forbidden",
    ),
    (
        FeatureId::GC02,
        "CREATE/DROP SCHEMA is outside the current catalog claim (graph-schema vs graph-type vs graph)",
    ),
    (
        FeatureId::GC04,
        "CREATE GRAPH stays outside the current catalog claim (D1 single-graph embeddable cannot create a second graph); DROP GRAPH is the IM_DROP_GRAPH factory-reset extension, not GC04",
    ),
    (
        FeatureId::GC05,
        "CREATE GRAPH IF NOT EXISTS modifier remains outside the current catalog claim",
    ),
    (
        FeatureId::GS01,
        "SESSION SET <name> GRAPH binds a session-local graph parameter; D1 single-graph embeddable has one graph and no <graph expression>/CURRENT_GRAPH (GV60) or catalog to bind",
    ),
    (
        FeatureId::GS02,
        "SESSION SET <name> BINDING TABLE binds a typed binding-table reference value (GV61); the reference-type surface is deferred, so D1 has no spelling to bind",
    ),
    (
        FeatureId::GS05,
        "SESSION RESET SCHEMA resets the session schema; D1 single-graph embeddable has no schema layer (section 4.2.5.1) to reset",
    ),
    (
        FeatureId::GS06,
        "SESSION RESET GRAPH resets the session graph; D1 single-graph embeddable has exactly one graph and no working-graph switch to reset",
    ),
    (
        FeatureId::GS10,
        "session-local binding table parameters from subqueries depend on GS02 (binding-table parameters) and a procedure body; both deferred under D1",
    ),
    (
        FeatureId::GS11,
        "session-local value parameters from subqueries depend on procedure-body support; SESSION SET VALUE restricts the RHS to a value expression with no row context",
    ),
    (
        FeatureId::GS12,
        "session-local graph parameters from simple graph expressions/references depend on GS01 (graph parameters); D1-blocked",
    ),
    (
        FeatureId::GS13,
        "session-local binding table parameters from simple expressions/references depend on GS02 (binding-table parameters); deferred",
    ),
    (
        FeatureId::GS14,
        "selene-db evaluates SESSION SET VALUE against an empty binding so the RHS is restricted to a <value specification> (literal/parameter); the full <value expression> form (GS14) is not claimed",
    ),
    (
        FeatureId::GT03,
        "multi-graph transactions are out of scope under the D1 single-graph embeddable model",
    ),
    (
        FeatureId::GV15,
        "256-bit unsigned integers are not represented in Value v1",
    ),
    (
        FeatureId::GV16,
        "256-bit signed integers are not represented in Value v1",
    ),
    (FeatureId::GV20, "FLOAT16 remains deferred"),
    (
        FeatureId::GV22,
        "specified floating precision syntax is deferred",
    ),
    (
        FeatureId::GV23,
        "REAL/DOUBLE synonyms require preserving original type spelling through the flagger before claiming",
    ),
    (FeatureId::GV25, "FLOAT128 is deferred"),
    (FeatureId::GV26, "FLOAT256 is deferred"),
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

mod annex_b;

pub use annex_b::{ANNEX_B_REGISTER, AnnexBId, ImplDefinedChoice};

/// True when `id` is in the supported feature set.
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

/// Return the non-support rationale for a referenced feature ID.
pub fn non_supported_rationale(id: FeatureId) -> Option<&'static str> {
    NOT_SUPPORTED_RATIONALE
        .iter()
        .find_map(|(feature, rationale)| (*feature == id).then_some(*rationale))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn supported_features_has_no_duplicates() {
        // CORE-15: a duplicated feature in SUPPORTED would silently inflate the
        // conformance claim count and is otherwise invisible.
        let mut seen = HashSet::new();
        for feature in SUPPORTED_FEATURES {
            assert!(
                seen.insert(*feature),
                "{feature} appears more than once in SUPPORTED_FEATURES"
            );
        }
    }

    #[test]
    fn not_supported_rationale_has_no_duplicates() {
        let mut seen = HashSet::new();
        for (feature, _) in NOT_SUPPORTED_RATIONALE {
            assert!(
                seen.insert(*feature),
                "{feature} appears more than once in NOT_SUPPORTED_RATIONALE"
            );
        }
    }

    #[test]
    fn supported_and_not_supported_are_disjoint() {
        // CORE-15: SUPPORTED ∩ NOT_SUPPORTED = ∅ — a feature cannot be both
        // claimed and rationalized-as-unclaimed.
        let supported: HashSet<FeatureId> = SUPPORTED_FEATURES.iter().copied().collect();
        for (feature, _) in NOT_SUPPORTED_RATIONALE {
            assert!(
                !supported.contains(feature),
                "{feature} is in BOTH SUPPORTED_FEATURES and NOT_SUPPORTED_RATIONALE"
            );
        }
    }

    #[test]
    fn ge08_is_reference_parameters_referenced_only() {
        // CONFORMANCE-00: GE08 is ISO Annex D Table D.1 row 77 / §17.7
        // "Reference parameters" — NOT a CAST feature. It must carry the correct
        // ISO name, must NOT be claimed (reference parameters are unimplemented),
        // and — having no syntactic surface to reject — must NOT be in
        // NOT_SUPPORTED_RATIONALE (which is reserved for parser-rejected
        // features). It surfaces as the "referenced" status instead.
        assert_eq!(name_of(FeatureId::GE08), Some("Reference parameters"));
        assert!(
            !is_supported(FeatureId::GE08),
            "GE08 (Reference parameters) is not implemented and must not be claimed"
        );
        assert!(
            non_supported_rationale(FeatureId::GE08).is_none(),
            "GE08 has no parser surface to reject; it is referenced-only, not rationalized"
        );
    }

    #[test]
    fn ga05_cast_specification_is_supported() {
        // CONFORMANCE-00 (Codex review follow-up): GA05 "Cast specification"
        // (Annex D row 53 / §20.8) is the real ISO feature for CAST. Per ISO
        // Annex A item 52, without GA05 "conforming GQL language shall not
        // contain a <cast specification>" — CAST is gated behind GA05, not
        // baseline. selene-db implements the cast construct, so it CLAIMS GA05
        // (and so GA05 carries no non-supported rationale).
        assert_eq!(name_of(FeatureId::GA05), Some("Cast specification"));
        assert!(
            is_supported(FeatureId::GA05),
            "GA05 is claimed: selene-db implements <cast specification>"
        );
        assert!(
            non_supported_rationale(FeatureId::GA05).is_none(),
            "GA05 is supported, so it has no non-supported rationale"
        );
    }

    #[test]
    fn gg21_explicit_key_label_sets_is_claimed() {
        // 813: GG21 "Explicit element type key label sets" is now CLAIMED. The
        // type-DDL grammar parses the explicit `<...type key label set>` (the
        // `=>` <implies> marker, ISO §18.2/18.3) and the flagger stamps it. A
        // claimed feature carries no non-supported rationale.
        assert_eq!(
            name_of(FeatureId::GG21),
            Some("Explicit element type key label sets")
        );
        assert!(
            is_supported(FeatureId::GG21),
            "GG21 is claimed: the explicit key-label-set `=>` syntax is parsed and flagged"
        );
        assert!(
            non_supported_rationale(FeatureId::GG21).is_none(),
            "GG21 is supported, so it has no non-supported rationale"
        );
        // §24.7 implied-feature-relationships: GG21 implies GG02 ("Graph with a
        // closed graph type"). The implication is satisfied — GG02 is claimed —
        // so claiming GG21 is implication-consistent and not a phantom claim.
        assert!(
            is_supported(FeatureId::GG02) && is_supported(FeatureId::GG20),
            "GG21 implies GG02 (closed graph type); GG02 + GG20 stay claimed"
        );
    }

    #[test]
    fn at_least_one_graph_type_feature_is_claimed() {
        // CORE-15: ISO §4 requires at least one of GG01 (open graph) or GG02
        // (closed graph). selene-db claims both, but the floor is the OR.
        assert!(
            is_supported(FeatureId::GG01) || is_supported(FeatureId::GG02),
            "neither GG01 (open graph) nor GG02 (closed graph) is claimed"
        );
    }

    #[test]
    fn every_referenced_feature_resolves_by_string() {
        // Round-trips the stable string ABI both ways.
        for (feature, _) in REFERENCED_FEATURES {
            assert_eq!(feature_id_from_str(feature.as_str()), Some(*feature));
            assert!(name_of(*feature).is_some(), "{feature} has no display name");
        }
    }

    #[test]
    fn annex_b_register_carries_no_pack_or_spec_05_residue() {
        // CORE-01: post-#196 the procedure-pack model is gone. No Annex B entry
        // may still name "pack" or point at the deleted spec 05 / spec 15.
        for (id, choice) in ANNEX_B_REGISTER {
            let haystacks = [choice.choice, choice.settled_in];
            for text in haystacks {
                let lower = text.to_ascii_lowercase();
                assert!(
                    !lower.contains("pack"),
                    "Annex B {} still references a 'pack': {text:?}",
                    id.as_str()
                );
                assert!(
                    !lower.contains("spec 05") && !lower.contains("spec 15"),
                    "Annex B {} still points at a deleted spec: {text:?}",
                    id.as_str()
                );
            }
        }
    }

    #[test]
    fn annex_b_register_has_no_duplicate_ids() {
        let mut seen = HashSet::new();
        for (id, _) in ANNEX_B_REGISTER {
            assert!(
                seen.insert(*id),
                "{} appears more than once in ANNEX_B_REGISTER",
                id.as_str()
            );
        }
    }
}
