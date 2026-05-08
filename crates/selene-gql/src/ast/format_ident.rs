//! Identifier-quoting policy and reserved-word set for the read-side
//! pretty-printer.
//!
//! Lifted out of `format.rs` to keep that file under the 700-LOC cap
//! (CLAUDE.md hard rule #5). The constants and helpers here are private
//! to `selene_gql::ast` and consumed only by `format.rs`.

use selene_core::IStr;

/// Aggregate-op keywords reserved by the `aggregate_expr` grammar rule.
///
/// Per `parser/grammar.pest` line 454, these tokens MUST appear bare in
/// function-call position so the parser can route the call through the
/// aggregate path (which accepts `*` and `DISTINCT`). Quoting any of them
/// rewrites the parse from aggregate to a generic function call and the
/// argument list shape diverges. [`fmt_call_segment`] consults this set
/// to opt out of quoting in the call-name context.
const AGGREGATE_OPS: &[&str] = &[
    "AVERAGE",
    "AVG",
    "COLLECT",
    "COLLECT_LIST",
    "COUNT",
    "MAX",
    "MIN",
    "STDDEV_POP",
    "STDDEV_SAMP",
    "SUM",
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
#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "ACYCLIC", "AFTER", "ALL", "ALL_DIFFERENT", "AND", "ANY", "ARRAY", "AS",
    "ASC", "AT", "AVERAGE", "AVG", "BETWEEN", "BIGINT", "BINDING", "BINDINGS",
    "BOOL", "BOOLEAN", "BOTH", "BY", "BYTEA", "BYTES", "CALL", "CASE", "CAST",
    "COLLECT", "COLLECT_LIST", "COMMIT", "CONNECTING", "CONTAINS", "COUNT",
    "CREATE", "DATE", "DATETIME", "DAY", "DEC", "DECIMAL", "DEFAULT", "DELETE",
    "DESC", "DESTINATION", "DETACH", "DICTIONARY", "DIFFERENT", "DIRECTED",
    "DISTINCT", "DOUBLE", "DROP", "DURATION", "EDGE", "EDGES", "ELEMENTS",
    "ELSE", "ENCODING", "END", "ENDS", "EXCEPT", "EXECUTE", "EXISTS", "EXTENDS",
    "FALSE", "FILL", "FILTER", "FINISH", "FIRST", "FLOAT", "FOR", "FROM",
    "GRANT", "GRAPH", "GROUP", "HAVING", "HOUR", "IF", "IMMUTABLE", "IN",
    "INDEX", "INDEXED", "INSERT", "INT", "INTEGER", "INTERSECT", "INTERVAL",
    "IS", "KEEP", "LABELED", "LABELS", "LAST", "LEADING", "LET", "LIKE",
    "LIMIT", "LIST", "LOCAL", "MATCH", "MATERIALIZED", "MAX", "MERGE", "MIN",
    "MINUTE", "MONTH", "NEXT", "NFC", "NFD", "NFKC", "NFKD", "NO", "NODE",
    "NODETACH", "NONE", "NORMALIZED", "NOT", "NOTHING", "NULL", "NULLS", "OF",
    "OFFSET", "ON", "ONLY", "OPTIONAL", "OR", "ORDER", "OTHERWISE", "PASSWORD",
    "PATH", "PRECISION", "PROCEDURE", "PROPERTY_EXISTS", "REAL", "RECORD",
    "REDUCE", "REMOVE", "REPEATABLE", "REPLACE", "RETURN", "REVOKE", "ROLE",
    "ROLLBACK", "SAME", "SEARCHABLE", "SECOND", "SELECT", "SET", "SHORTEST",
    "SHOW", "SIGNED", "SIMPLE", "SINGLE", "SKIP", "SMALLINT", "SOURCE",
    "START", "STARTS", "STDDEV_POP", "STDDEV_SAMP", "STRICT", "STRING", "SUM",
    "THEN", "TIME", "TIMESTAMP", "TO", "TRAIL", "TRAILING", "TRANSACTION",
    "TRIGGER", "TRIGGERS", "TRIM", "TRUE", "TYPE", "TYPED", "TYPES", "UINT",
    "UNION", "UNIQUE", "UNKNOWN", "UNWIND", "USER", "UUID", "VARCHAR",
    "VECTOR", "VIEW", "VIEWS", "WALK", "WARN", "WHEN", "WHERE", "WITH", "XOR",
    "YEAR", "YIELD", "ZONED",
];

/// Format an identifier slot (binding name, alias name, property key).
///
/// Returns the bare identifier when it is a simple ASCII ident and not a
/// grammar-reserved keyword; otherwise returns the double-quoted form
/// with embedded `"` escaped as `""`.
pub(super) fn fmt_ident(value: IStr) -> String {
    let value = value.as_str();
    if is_simple_ident(value) && !KEYWORDS.contains(&value.to_ascii_uppercase().as_str()) {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Format a function-call name segment.
///
/// Same quoting policy as [`fmt_ident`], minus the aggregate-op tokens.
/// The grammar's `aggregate_expr` rule (see `parser/grammar.pest`) demands
/// the bare keyword (case-insensitive) so it can recognise `count(*)` as
/// the COUNT aggregate; quoting any of those names breaks the parse.
/// Aggregate ops are not safe identifier names anyway — they are
/// grammar-reserved at every site where this function is consulted.
pub(super) fn fmt_call_segment(value: IStr) -> String {
    let value = value.as_str();
    if is_simple_ident(value) {
        let upper = value.to_ascii_uppercase();
        let is_aggregate = AGGREGATE_OPS.contains(&upper.as_str());
        let is_keyword = KEYWORDS.contains(&upper.as_str());
        if is_aggregate || !is_keyword {
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
