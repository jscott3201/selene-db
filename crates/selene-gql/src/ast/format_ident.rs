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
// This intentionally mirrors the parser's current reserved/pre-reserved surface
// plus a few context-sensitive implementation words that have historically
// needed quoting to keep formatted ASTs round-trip-stable.
#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "ABSTRACT", "ABS", "ACOS", "ACYCLIC", "AGGREGATE", "AGGREGATES", "ALL",
    "ALL_DIFFERENT", "ALTER", "AND", "ANY", "ARRAY", "AS", "ASC", "ASCENDING",
    "ASIN", "AT", "ATAN", "AVG", "BIG", "BIGINT", "BINARY", "BINDING",
    "BINDINGS", "BOOL", "BOOLEAN", "BOTH", "BTRIM", "BY", "BYTE_LENGTH",
    "BYTEA", "BYTES", "CALL", "CARDINALITY", "CASE", "CAST", "CATALOG",
    "CEIL", "CEILING", "CHAR", "CHAR_LENGTH", "CHARACTER_LENGTH",
    "CHARACTERISTICS", "CLEAR", "CLONE", "CLOSE", "COALESCE", "COLLECT_LIST",
    "COMMIT", "CONNECTING", "CONSTRAINT", "CONTAINS", "COPY", "COS", "COSH",
    "COT", "COUNT", "CREATE", "CURRENT_DATE", "CURRENT_GRAPH",
    "CURRENT_PROPERTY_GRAPH", "CURRENT_ROLE", "CURRENT_SCHEMA",
    "CURRENT_TIME", "CURRENT_TIMESTAMP", "CURRENT_USER", "DATA", "DATE",
    "DATETIME", "DAY", "DEC", "DECIMAL", "DEFAULT", "DEGREES", "DELETE",
    "DESC", "DESCENDING", "DESTINATION", "DETACH", "DICTIONARY", "DIFFERENT",
    "DIRECTED", "DIRECTORY", "DISTINCT", "DOUBLE", "DROP", "DRYRUN",
    "DURATION", "DURATION_BETWEEN", "EDGE", "EDGES", "ELEMENT_ID",
    "ELEMENTS", "ELSE", "ENCODING", "END", "ENDS", "EXACT", "EXCEPT",
    "EXISTING", "EXISTS", "EXP", "EXTENDS", "FALSE", "FILL", "FILTER",
    "FINISH", "FIRST", "FLOAT", "FLOAT16", "FLOAT32", "FLOAT64",
    "FLOAT128", "FLOAT256", "FLOOR", "FOR", "FROM", "FUNCTION", "GQLSTATUS",
    "GRANT", "GRAPH", "GROUP", "HAVING", "HOME_GRAPH", "HOME_PROPERTY_GRAPH",
    "HOME_SCHEMA", "HOUR", "IF", "IMMUTABLE", "IMPLIES", "IN", "INDEX",
    "INDEXED", "INFINITY", "INSERT", "INSTANT", "INT", "INT8", "INT16",
    "INT32", "INT64", "INT128", "INT256", "INTEGER", "INTEGER8", "INTEGER16",
    "INTEGER32", "INTEGER64", "INTEGER128", "INTEGER256", "INTERSECT",
    "INTERVAL", "IS", "KEEP", "LABELED", "LABELS", "LAST", "LEADING",
    "LEFT", "LET", "LIKE", "LIMIT", "LIST", "LN", "LOCAL",
    "LOCAL_DATETIME", "LOCAL_TIME", "LOCAL_TIMESTAMP", "LOG", "LOG10",
    "LOWER", "LTRIM", "MATCH", "MAX", "MERGE", "MIN", "MINUTE", "MOD",
    "MONTH", "NEXT", "NFC", "NFD", "NFKC", "NFKD", "NO", "NODE",
    "NODETACH", "NONE", "NORMALIZE", "NORMALIZED", "NOT", "NOTHING",
    "NULL", "NULLIF", "NULLS", "NUMBER", "NUMERIC", "OCTET_LENGTH", "OF",
    "OFFSET", "ON", "ONLY", "OPEN", "OPTIONAL", "OR", "ORDER", "ORDINALITY",
    "OTHERWISE", "PARAMETER", "PARAMETERS", "PARTITION", "PATH", "PATH_LENGTH",
    "PATHS", "PERCENTILE_CONT", "PERCENTILE_DISC", "POWER", "PRECISION",
    "PROCEDURE", "PRODUCT", "PROJECT", "PROPERTY_EXISTS", "QUERY", "RADIANS",
    "REAL", "RECORD", "RECORDS", "REDUCE", "REFERENCE", "REMOVE", "RENAME",
    "REPEATABLE", "REPLACE", "RESET", "RETURN", "REVOKE", "RIGHT",
    "ROLLBACK", "RTRIM", "SAME", "SCHEMA", "SEARCHABLE", "SECOND", "SELECT",
    "SESSION", "SESSION_USER", "SET", "SHORTEST", "SHOW", "SIGNED", "SIMPLE",
    "SIN", "SINH", "SINGLE", "SIZE", "SKIP", "SMALL", "SMALLINT", "SOURCE",
    "SQRT", "START", "STARTS", "STDDEV_POP", "STDDEV_SAMP", "STRICT",
    "STRING", "SUBSTRING", "SUM", "SYSTEM_USER", "TAN", "TANH", "TEMPORAL",
    "THEN", "TIME", "TIMESTAMP", "TO", "TRAIL", "TRAILING", "TRANSACTION",
    "TRIM", "TRUE", "TYPE", "TYPED", "TYPES", "UBIGINT", "UINT", "UINT8",
    "UINT16", "UINT32", "UINT64", "UINT128", "UINT256", "UNION", "UNIQUE",
    "UNIT", "UNKNOWN", "UNSIGNED", "UNWIND", "UPPER", "USE", "USMALLINT",
    "UUID", "VALUES", "VARBINARY", "VARCHAR", "VARIABLE", "WALK", "WARN",
    "WHEN", "WHERE", "WHITESPACE", "WITH", "WITHOUT", "XOR", "YEAR", "YIELD",
    "ZONED", "ZONED_DATETIME", "ZONED_TIME",
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
    "EXPLAIN", "INDEXES", "PROCEDURES", "TRANSACTIONS", "VALUE",
];

/// Format an identifier slot (binding name, alias name, property key).
///
/// Returns the bare identifier when it is a simple ASCII ident and not a
/// grammar-reserved keyword; otherwise returns the double-quoted form
/// with embedded `"` escaped as `""`.
pub(crate) fn fmt_ident(value: DbString) -> String {
    let value = value.as_str();
    let upper = value.to_ascii_uppercase();
    if is_simple_ident(value) && !is_identifier_keyword(&upper) {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Format an identifier in expression-head position.
///
/// Double quotes are string literals in expression slots, so variables and
/// generic function names must use backticks when bare spelling is unsafe.
pub(crate) fn fmt_expr_ident(value: DbString) -> String {
    let value = value.as_str();
    let upper = value.to_ascii_uppercase();
    if is_simple_ident(value) && !KEYWORDS.contains(&upper.as_str()) {
        return value.to_owned();
    }
    fmt_backtick_ident(value)
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
    fmt_backtick_ident(value)
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

fn fmt_backtick_ident(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}
