//! Identifier-quoting policy and reserved-word set for the read-side
//! pretty-printer.
//!
//! Lifted out of `format.rs` to keep that file under the 700-LOC cap
//! (CLAUDE.md hard rule #5). The constants and helpers here are private
//! to `selene_gql::ast` and consumed only by `format.rs`.

use selene_core::DbString;

/// Aggregate-op keywords reserved by the `aggregate_expr` grammar rule.
///
/// Tokens accepted by the `aggregate_expr` grammar rule MUST appear bare in
/// function-call position so the parser can route the call through the
/// aggregate path (which accepts `*` and `DISTINCT`). Quoting any of them
/// rewrites the parse from aggregate to a generic function call and the
/// argument list shape diverges. [`fmt_call_segment`] consults this set
/// to opt out of quoting in the call-name context.
const AGGREGATE_OPS: &[&str] = &[
    "AVG",
    "COLLECT_LIST",
    "COUNT",
    "MAX",
    "MIN",
    "PERCENTILE_CONT",
    "PERCENTILE_DISC",
    "STDDEV_POP",
    "STDDEV_SAMP",
    "SUM",
];

/// Reserved scalar-function heads that remain bare in function-call position.
///
/// These are grammar keywords, so identifier slots must quote them, but the
/// parser has dedicated primary-expression rules that accept the bare call
/// form. Keeping them bare preserves ISO-style formatting such as
/// `LEFT('abc', 2)` and source-only `TRIM(' x ')`.
const KEYWORD_FUNCTION_CALLS: &[&str] = &[
    "ABS",
    "ACOS",
    "ASIN",
    "ATAN",
    "BTRIM",
    "BYTE_LENGTH",
    "CARDINALITY",
    "CEIL",
    "CEILING",
    "CHAR_LENGTH",
    "CHARACTER_LENGTH",
    "COS",
    "COSH",
    "COT",
    "COALESCE",
    "DATE",
    "DATETIME",
    "DEGREES",
    "DURATION",
    "ELEMENT_ID",
    "ELEMENTS",
    "EXP",
    "FLOOR",
    "LABELS",
    "LEFT",
    "LOCAL_DATETIME",
    "LOCAL_TIME",
    "LN",
    "LOG",
    "LOG10",
    "LOWER",
    "LTRIM",
    "MOD",
    "NULLIF",
    "OCTET_LENGTH",
    "PATH_LENGTH",
    "POWER",
    "RADIANS",
    "RIGHT",
    "RTRIM",
    "SIN",
    "SINH",
    "SIZE",
    "SQRT",
    "TAN",
    "TANH",
    "TIME",
    "TRIM",
    "UPPER",
    "ZONED_DATETIME",
    "ZONED_TIME",
];

/// Reserved-word set against which [`fmt_ident`] decides whether to quote.
///
/// Derived from every `^"WORD"` keyword token referenced in
/// `crates/selene-gql/src/parser/grammar.pest`. The list is intentionally
/// over-conservative: any identifier whose uppercase form matches an entry
/// is quoted in the formatted output. Over-quoting is round-trip-safe
/// because [`crate::parser::builders::decode_ident_like`] strips the
/// surrounding quotes and returns the same case-preserved bytes for both
/// `name` and `"name"`.
///
/// Codex P2 on PR #24 caught the previous, much shorter list (24 entries):
/// identifiers like `DISTINCT`, `WITH`, `ASC` could be emitted bare and
/// then re-parse as keywords, breaking the §D3 round-trip property.
// NOTE (2026-05-28 grammar purge): the 16 keywords whose grammar rules were
// removed for ISO faithfulness (LIKE, BETWEEN, GRANT, REVOKE, ROLE, USER,
// PASSWORD, PROCEDURE, TRIGGER, TRIGGERS, MATERIALIZED, VIEW, VIEWS, AT, AFTER,
// EXECUTE) were dropped from this set so it again mirrors grammar.pest.
#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "ABS", "ACOS", "ACYCLIC", "ALL", "ALL_DIFFERENT", "AND", "ANY", "ARRAY",
    "AS", "ASC", "ASIN", "ATAN", "AVG", "BIGINT", "BINDING", "BINDINGS",
    "BOOL", "BOOLEAN", "BOTH", "BTRIM", "BY", "BYTE_LENGTH", "BYTEA",
    "BYTES", "CALL", "CARDINALITY", "CASE", "CAST", "CEIL", "CEILING",
    "CHAR_LENGTH", "CHARACTER_LENGTH", "COLLECT_LIST", "COMMIT", "CONNECTING",
    "CONTAINS", "COS", "COSH", "COT", "COALESCE", "COUNT", "CREATE",
    "CURRENT_DATE", "CURRENT_TIME", "CURRENT_TIMESTAMP", "DATE", "DATETIME",
    "DAY", "DEC", "DECIMAL", "DEFAULT", "DEGREES", "DELETE", "DESC",
    "DESTINATION", "DETACH", "DICTIONARY", "DIFFERENT", "DIRECTED", "DISTINCT",
    "DOUBLE", "DROP", "DURATION", "DURATION_BETWEEN", "EDGE", "EDGES",
    "ELEMENT_ID", "ELEMENTS", "ELSE", "ENCODING", "END", "ENDS", "EXCEPT",
    "EXISTS", "EXP", "EXTENDS", "FALSE", "FILL", "FILTER", "FINISH",
    "FIRST", "FLOAT", "FLOOR", "FOR", "FROM",
    "GRAPH", "GROUP", "HAVING", "HOUR", "IF", "IMMUTABLE", "IN", "INDEX",
    "INDEXED", "INSERT", "INT", "INTEGER", "INTERSECT", "INTERVAL", "IS",
    "KEEP", "LABELED", "LABELS", "LAST", "LEADING", "LEFT", "LET", "LIMIT",
    "LIST", "LN", "LOCAL", "LOCAL_DATETIME", "LOCAL_TIME", "LOCAL_TIMESTAMP",
    "LOG", "LOG10", "LOWER", "LTRIM", "MATCH", "MAX", "MERGE", "MIN",
    "MINUTE", "MOD", "MONTH", "NEXT", "NFC", "NFD", "NFKC", "NFKD", "NO",
    "NODE", "NODETACH", "NONE", "NORMALIZE", "NORMALIZED", "NOT", "NOTHING",
    "NULL", "NULLIF", "NULLS", "OCTET_LENGTH", "OF", "OFFSET", "ON",
    "ONLY", "OPTIONAL", "OR", "ORDER", "OTHERWISE", "PATH", "PATH_LENGTH", "POWER", "PRECISION",
    "PROPERTY_EXISTS", "RADIANS", "REAL", "RECORD", "REDUCE", "REMOVE",
    "REPEATABLE", "REPLACE", "RETURN", "RIGHT", "ROLLBACK", "RTRIM", "SAME",
    "SEARCHABLE", "SECOND", "SELECT", "SET", "SHORTEST", "SHOW", "SIGNED",
    "SIMPLE", "SIN", "SINH", "SINGLE", "SIZE", "SKIP", "SMALLINT", "SOURCE",
    "SQRT", "START", "STARTS", "STDDEV_POP", "STDDEV_SAMP", "STRICT", "STRING",
    "SUM", "TAN", "TANH", "THEN", "TIME", "TIMESTAMP", "TO", "TRAIL",
    "TRAILING", "TRANSACTION", "TRIM", "TRUE", "TYPE", "TYPED", "TYPES",
    "UINT", "UNION", "UNIQUE", "UNKNOWN", "UNWIND", "UPPER", "UUID",
    "VARCHAR", "WALK", "WARN", "WHEN", "WHERE", "WITH", "WITHOUT", "XOR", "YEAR",
    "YIELD", "ZONED", "ZONED_DATETIME", "ZONED_TIME",
];

/// Contextual keyword tokens that must be quoted in identifier slots.
///
/// These tokens are not globally reserved by the parser because each appears
/// only in a specific grammar context (`EXPLAIN`, `SHOW INDEXES`,
/// `PERCENTILE_CONT(...)`, ...). A bare identifier with the
/// same spelling can still parse as an identifier, but emitting it bare hides
/// its identifier role in formatted output and leaves future grammar additions
/// room to break round trips. Keep them out of [`KEYWORDS`] so function-call
/// formatting can continue to apply call-specific rules.
#[rustfmt::skip]
const CONTEXTUAL_IDENTIFIER_KEYWORDS: &[&str] = &[
    "EXPLAIN", "INDEXES", "PERCENTILE_CONT", "PERCENTILE_DISC", "PROCEDURES",
    "TRANSACTIONS", "VALUE",
];

/// Format an identifier slot (binding name, alias name, property key).
///
/// Returns the bare identifier when it is a simple ASCII ident and not a
/// grammar-reserved keyword; otherwise returns the double-quoted form
/// with embedded `"` escaped as `""`.
pub(super) fn fmt_ident(value: DbString) -> String {
    let value = value.as_str();
    let upper = value.to_ascii_uppercase();
    if is_simple_ident(value) && !is_identifier_keyword(&upper) {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Format a function-call name segment.
///
/// Same quoting policy as [`fmt_ident`], minus aggregate-op tokens and reserved
/// scalar-function heads that the parser accepts in bare call position.
/// The grammar's `aggregate_expr` rule (see `parser/grammar.pest`) demands
/// the bare keyword (case-insensitive) so it can recognise `count(*)` as
/// the COUNT aggregate; quoting any of those names breaks the parse.
/// Aggregate ops are not safe identifier names anyway — they are
/// grammar-reserved at every site where this function is consulted.
pub(super) fn fmt_call_segment(value: DbString) -> String {
    let value = value.as_str();
    if is_simple_ident(value) {
        let upper = value.to_ascii_uppercase();
        let is_aggregate = AGGREGATE_OPS.contains(&upper.as_str());
        let is_keyword_function = KEYWORD_FUNCTION_CALLS.contains(&upper.as_str());
        let is_keyword = KEYWORDS.contains(&upper.as_str());
        if is_aggregate || is_keyword_function || !is_keyword {
            return value.to_owned();
        }
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Escape a string literal's body for re-emission in single quotes.
pub(super) fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\'', "''")
}

fn is_simple_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic()) && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

fn is_identifier_keyword(upper: &str) -> bool {
    KEYWORDS.contains(&upper) || CONTEXTUAL_IDENTIFIER_KEYWORDS.contains(&upper)
}
