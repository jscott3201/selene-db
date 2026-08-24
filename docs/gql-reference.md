# GQL reference

This document is the query-author's reference for the GQL surface that
selene-db exposes. It assumes you have read the README quickstart and can
build a `SharedGraph` plus an `EmptyProcedureRegistry`.

The current engine implements selected ISO/IEC 39075:2024 GQL syntax and
semantics plus namespaced extensions. It does not make a blanket minimum- or
selected-profile conformance claim. The parser keeps a strict GQL boundary: no
Cypher, SQL, or SPARQL grammar. The generated profile independently records
Flagger admission, runtime status, formal claim state, and evidence; none of
those fields alone is formal 2.0 claim evidence. See the [2.0 conformance
policy](v2/conformance-policy.md).

For the engine architecture see [`architecture.md`](architecture.md). For
durability and recovery see
[`persistence-and-recovery.md`](persistence-and-recovery.md). For the native
graph algorithms exposed via `CALL algo.*` see
[`graph-algorithms.md`](graph-algorithms.md).

The Rust API used in every example below is:

```rust
use selene_gql::{
    EmptyProcedureRegistry, StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;

let registry = EmptyProcedureRegistry;
let statement = parse(source)?;
let analyzed = analyze(statement, &registry, None)?;
let planned = plan(&analyzed, &registry)?;
let mut session = selene_gql::Session::new(&graph);
let output = execute_statement(&planned, &mut session, &registry)?;
```

The `optimize` pass is included internally by `plan`'s lowering pipeline;
callers who want manual control can call `optimize(plan, &OptimizeContext)`
between `plan` and `execute_statement`.

---

## 1. Current implementation inventory

`selene_profile::capabilities()` is the typed, allocation-free view of the
generated profile's runtime-support inventory. The table below summarizes
current behavior; “Full” and “Partial” describe implementation coverage in that
row, not formal conformance status.

| Group | Coverage | Notes |
|---|---|---|
| Read query (`MATCH`, `OPTIONAL MATCH`, `WHERE`, `RETURN`, `WITH`, `FOR`, `ORDER BY`, `LIMIT`, `OFFSET`, `DISTINCT`) | Full | The pipeline form is canonical; `SELECT ... FROM` desugars at the AST level. |
| Set composition (`UNION`, `EXCEPT`, `INTERSECT`, `OTHERWISE`, chained `NEXT`) | Full | `OTHERWISE` is `GQ02`; `UNION`, `EXCEPT`, and `INTERSECT` support `ALL` / `DISTINCT` variants (`GQ03`-`GQ07`). |
| Aggregation (`count`, `sum`, `avg`, `min`, `max`, `collect`, `stddev_pop`, `stddev_samp`) | Full | `GROUP BY` is runtime-supported as feature `GQ15`. |
| Mutation (`INSERT`, `SET`, `REMOVE`, `DELETE`, `DETACH DELETE`) | Full | `MutationPipeline` accepts an optional terminator (`RETURN` or `FINISH`). `MERGE` remains deferred. |
| DDL (`CREATE/DROP SCHEMA`, `CREATE/DROP GRAPH`, `CREATE/DROP/ALTER NODE TYPE`, `CREATE/DROP/ALTER EDGE TYPE`, `SHOW NODE TYPES`, `SHOW EDGE TYPES`) | Partial | Schema and open-graph management (`GC01`, `GC02`, `GC04`, `GC05`, `GG01`) execute through the `selene-db` facade catalog service. Closed graph types (`GG02`), inline and `LIKE` graph types (`GG03`, `GG04`), graph sources (`GG05`), and `CREATE/DROP GRAPH TYPE` are rejected with `42N01`. Both additive `ALTER` forms are implementation-defined surfaces. Type DDL is Flagger-accepted as part of the directly selected parser surface even though the runtime inventory reports `GG02`, `GG20`, and `GG21` as unsupported. |
| Procedure calls (`CALL ns.proc(args) YIELD col1, col2`, `CALL { ... }`) | Full | Named procedure calls are feature `GP04`; inline `CALL` query subqueries are runtime-supported as `GP01`-`GP03`. Procedure-local definitions remain out of scope. |
| Transaction control (`START TRANSACTION`, `COMMIT`, `ROLLBACK`) | Full | Feature `GT01`. Multi-graph transactions (`GT03`) are not runtime-supported. |
| Path patterns (variable-length, ANY/ALL SHORTEST, counted shortest) | Partial | `ANY`, `ANY SHORTEST`, `ALL`, `ALL SHORTEST`, and counted shortest path/group selectors are runtime-supported (`G015`-`G020`). Implementation-defined quantifier caps still apply to unbounded cyclic searches. |
| Predicates (`IS DIRECTED`, `IS LABELED`, `IS SOURCE/DESTINATION OF`, `ALL_DIFFERENT`, `SAME`, `PROPERTY_EXISTS`) | Full | Features `G110`-`G115`. |

Parser-observed capabilities whose generated Flagger disposition is `rejected`
fail during parsing. Flagger admission is not inferred from runtime status.

---

## 2. Data types

### Core scalar types implemented

| GQL type | Literal syntax | `Value` variant | Notes |
|:---|:---|:---|:---|
| `BOOLEAN` | `TRUE`, `FALSE`, `UNKNOWN` | `Value::Bool` | Three-valued logic applies to `=`, `<>`, comparisons, and Boolean composition. |
| `INTEGER` / `INT` | `42`, `-17`, `0`, `1_000` (underscores allowed) | `Value::Int` (i64) | Default integer is i64. Implementation-defined IA037 / ID028 set i64 default with i128 promotion when context demands. |
| `FLOAT` | `3.14`, `-0.5`, `1.0e6`, `2.5e-3` | `Value::Float` (f64) | IEEE 754 binary64 (feature `GA01`). |
| `STRING` | `'single quotes'`, `"double quotes"`, `` `accent quotes` ``, `'\n'` escapes | `Value::String(DbString)` | ISO single-, double-, and accent-quoted character strings. Doubled delimiters and backslash escapes are honored unless the literal uses `@` no-escape form. |

### Additional types in the current register

| GQL type | Literal syntax | `Value` variant | Feature |
|:---|:---|:---|:---|
| `INT8`, `INT16`, `INT32` | `CAST(x AS INT32)` | `Value::Int` | `GV02`, `GV04`, `GV07` |
| `UINT8`, `UINT16`, `UINT32`, `UINT64` | `CAST(x AS UINT32)` | `Value::Uint` | `GV01`, `GV03`, `GV06`, `GV11` |
| `INT128`, `UINT128` | `CAST(x AS INT128)`, `CAST(x AS UINT128)` | `Value::Int128`, `Value::Uint128` | `GV13`, `GV14` |
| `DECIMAL` | `CAST('1.23' AS DECIMAL)` | `Value::Decimal` (`rust_decimal::Decimal`) | `GV17`, 28 significant digits |
| `FLOAT32`, `REAL` | `1.5f`, `CAST(x AS REAL)` | `Value::Float32` | `GV21`, `GV23` |
| `FLOAT64`, `DOUBLE`, `DOUBLE PRECISION` | `CAST(x AS DOUBLE)` | `Value::Float` | `GV24`, `GV23` |
| `BYTES`, `BYTES(n)`, `BYTES(min,max)`, `BINARY(n)`, `VARBINARY(n)` | `CAST(x AS BYTES(2,4))` | `Value::Bytes` | `GV35`-`GV38`; fixed/variable bounded aliases canonicalize to `BYTES(min,max)` descriptors. |
| `DATE` | `DATE '2026-05-16'` | `Value::Date` | `GV39` |
| `LOCAL DATETIME` | `LOCAL DATETIME '2026-05-16T08:30:00'` | `Value::LocalDateTime` | `GV39` |
| `LOCAL TIME` | `LOCAL TIME '08:30:00'` | `Value::LocalTime` | `GV39` |
| `ZONED DATETIME` | `ZONED DATETIME '2026-05-16T08:30:00-07:00'` | `Value::ZonedDateTime` | `GV40` |
| `ZONED TIME` | `ZONED TIME '08:30:00-07:00'` | `Value::ZonedTime` | `GV40` |
| `DURATION` | `DURATION 'PT1H30M'` or `DURATION('1h30m')` | `Value::Duration` | `GV41` |
| `LIST<T>`, `ARRAY<T>`, postfix `T LIST` / `T ARRAY` | `[1, 2, 3]`, `CAST(x AS LIST<INTEGER>)` | `Value::List` | `GV50`; formatter canonicalizes to `LIST<...>`. |
| `PATH` | constructed by `MATCH` path variables | `Value::Path` | `GV55` |

String-source numeric casts follow the ISO signed/unsigned numeric literal image
rules. Digit separators and radix integer images are accepted where the target
feature is runtime-supported, for example `CAST('0x10' AS INTEGER)`,
`CAST('0o777' AS UINT64)`, and `CAST('0b1010' AS DECIMAL)`.

`NULL` and `UNKNOWN` are first-class. `NULL` represents missing data;
`UNKNOWN` is the three-valued-logic Boolean. Three-valued logic flows
through every Boolean operator (`AND`, `OR`, `XOR`, `NOT`) and through
the comparison family.

### Optional type surfaces outside current runtime support

Graph and binding-table reference type spellings (`GV60`-`GV61`), `FLOAT16` /
`FLOAT128` / `FLOAT256`, and 256-bit integers have generated non-support
rationales. A query that mentions one of these deferred types is rejected while
parsing.

Explicit value-type nullability (`GV90`), length-qualified byte-string types
(`GV36`-`GV38`), and `REAL` / `DOUBLE` synonyms (`GV23` / `GV24`) are
runtime-supported and covered by conformance corpus rows.

### Numeric literal forms

| Form | Example | Notes |
|---|---|---|
| Decimal integer | `1234`, `-1234`, `1_000_000` | i64 unless explicit cast. |
| Hex | `0xCAFE`, `-0x1A` | Feature `GL01`; normalizes to i64 unless explicitly cast. |
| Oct | `0o777` | Feature `GL02`; normalizes to i64 unless explicitly cast. |
| Bin | `0b1010` | Feature `GL03`; normalizes to i64 unless explicitly cast. |
| Exact decimal | `1.5`, `.5`, `1.`, `1.5M`, `1e2M` | Features `GL04`-`GL06`; lowers to `DECIMAL`. |
| Approximate float | `1e2`, `1.5F`, `1.5D`, `1e2F`, `1e2D` | Features `GL07`-`GL10` for suffix forms; lowers to f64. |

### Character string literal forms

| Form | Example | Notes |
|---|---|---|
| Single-quoted | `'Ada''s graph'`, `'line\nnext'` | Standard escaped form; doubled delimiters and backslash escapes are decoded. |
| Double-quoted | `"Ada ""graph"""`, `"line\nnext"` | Expression slots treat this as a string literal; identifier slots still use double quotes for delimited identifiers. |
| Accent-quoted | `` `Ada graph` ``, `` `line\nnext` `` | Expression slots treat this as a string literal; identifier slots still use accent quotes for delimited identifiers. Doubled grave accents escape a literal grave accent. |
| No escape | `@'path\raw'`, `@"path\raw"`, `` @`path\raw` `` | Feature `GL11`; backslashes are literal and the active quote delimiter cannot appear inside the body. |

### Identifiers and delimited identifiers

| Form | Example | Notes |
|---|---|---|
| Unquoted | `name`, `Person`, `customer_id` | Unicode letters / digits / underscore. Reserved keywords cannot appear unquoted. |
| Double-quoted | `"first name"`, `"with ""quote"""` | Spec form. `""` escapes a literal double quote. |
| Accent-quoted | `` `first name` `` | Spec form. Doubled grave accents escape a literal grave accent. |
| Property identifier | `n.date`, `{date: 1}` | Keywords like `date`, `time`, `type` are valid property names without quoting. |

---

## 3. Reading data

The query pipeline form is canonical. A pipeline is one or more pipeline
statements that thread a binding table through transformations.

### `MATCH`

```gql
MATCH (p:Person {name: 'Ada'})-[r:KNOWS]->(q:Person)
RETURN p.name, q.name, r.since
```

```rust
let stmt = parse(
    "MATCH (p:Person {name: 'Ada'})-[r:KNOWS]->(q:Person) RETURN p.name, q.name, r.since"
)?;
let analyzed = analyze(stmt, &registry, None)?;
let planned = plan(&analyzed, &registry)?;
let output = execute_statement(&planned, &mut session, &registry)?;
```

Pattern syntax: `(node_var:Label {prop: value})-[edge_var:TYPE]->(node)`.
Edge direction tokens are `->` (right), `<-` (left), and `-` (any).
Bracketed edges may carry a variable, a label expression, a quantifier,
a property map, and an inline `WHERE`. Abbreviated forms (`->`, `<-`,
`-`) match any edge without binding (feature `G044`).

Label expressions support `:Foo|Bar` (or), `:Foo&Bar` (and), `:!Foo`
(not), and the `%` wildcard. Quantifiers (`*`, `+`, `?`, `{2,5}`,
`*2..`) control variable-length matching with an implementation-defined
upper bound of 100 (Annex B `IL018`).

Quantified edge variables are bound as path-ordered lists of edge references.
Property access over that binding maps across the list and preserves path
order:

```gql
MATCH (p:Person)-[r:KNOWS*1..3]->(friend)
RETURN r.score AS path_scores
```

Each row's `path_scores` value is a list. Missing edge properties become
`NULL` at the corresponding list position. `PROPERTY_EXISTS` remains scalar
over graph elements and records; use `FOR` when per-element existence checks are
needed.

### `OPTIONAL MATCH`

```gql
MATCH (p:Person {name: 'Ada'})
OPTIONAL MATCH (p)-[:KNOWS]->(friend)
RETURN p.name, friend.name
```

Optional match preserves `p` rows whose `friend` half is unmatched; the
unmatched `friend` is bound to `NULL`.

### `WHERE`

```gql
MATCH (p:Person)
WHERE p.age >= 18 AND p.country = 'NZ'
RETURN p.name
```

`WHERE` accepts any expression that evaluates to a Boolean (or three-valued
`UNKNOWN`). Inline `WHERE` inside a pattern node is also supported.

### `RETURN`

```gql
MATCH (p:Person)
RETURN DISTINCT p.country AS country, count(*) AS n
ORDER BY n DESC
LIMIT 10
```

Projections may be aliased (`expr AS name`). `RETURN *` returns every
in-scope binding. ISO defines `RETURN NO BINDINGS` only as an internal
specification device for transformations; Selene rejects it as user syntax.
Use `FINISH` for write pipelines that intentionally omit a result.

### `WITH`

```gql
MATCH (p:Person)-[:WORKS_AT]->(c:Company)
WITH c, count(*) AS headcount
WHERE headcount > 100
RETURN c.name, headcount
```

`WITH` introduces a projection-and-filter boundary: bindings beyond `WITH`
are exactly the projected aliases. `DISTINCT`, `GROUP BY`, `HAVING`, and
`WHERE` are all valid after `WITH`.

### `FOR`

```gql
FOR x IN [1, 2, 3, 4]
RETURN x * x AS squared

FOR x IN [1, 2, 3, 4] WITH ORDINALITY ord
RETURN x, ord

FOR x IN [1, 2, 3, 4] WITH OFFSET off
RETURN x, off
```

`FOR` is the ISO list-value row-expansion statement. It flattens a list
expression into row-per-element. The expression can be a list literal, a
list-typed property, or any expression evaluating to `LIST<T>`. `WITH
ORDINALITY` adds a one-based position column; `WITH OFFSET` adds a zero-based
position column.

### `ORDER BY`, `LIMIT`, `OFFSET`, `DISTINCT`

```gql
MATCH (p:Person)
RETURN p.name, p.age
ORDER BY p.age DESC NULLS LAST
SKIP 20
LIMIT 10
```

`OFFSET` and `SKIP` are synonyms. `NULLS FIRST` / `NULLS LAST` controls
null placement. `DISTINCT` works on `RETURN`, `WITH`, and the various
aggregate function calls.

### `LET` and `FOR`

```gql
LET total = 0, prefix = 'k_'
RETURN total, prefix
```

`LET` binds value variables. `FOR x IN expr` iterates over a list-typed
expression using ISO row-expansion syntax.

### `FILTER`

```gql
MATCH (p:Person)
FILTER p.age >= 18
RETURN p.name
```

`FILTER` (with optional `WHERE` keyword) is the pipeline form of `WHERE`.

### Predicates

| Predicate | Example | Feature |
|---|---|---|
| `IS NULL`, `IS NOT NULL` | `WHERE p.name IS NOT NULL` | mandatory |
| `IS TRUE / FALSE / UNKNOWN` | `WHERE x IS TRUE` | mandatory |
| `IS TYPED <type>` | `WHERE x IS TYPED INTEGER` | mandatory |
| `IS NORMALIZED NFC/NFD/NFKC/NFKD` | `WHERE s IS NORMALIZED NFC` | mandatory |
| `IS DIRECTED` | `WHERE r IS DIRECTED` | `G110` |
| `IS LABELED :Person` | `WHERE n IS LABELED :Person` | `G111` |
| `IS SOURCE OF`, `IS DESTINATION OF` | `WHERE n IS SOURCE OF r` | `G112` |
| `ALL_DIFFERENT(a, b, c, ...)` | `WHERE ALL_DIFFERENT(p1, p2, p3)` | `G113` |
| `SAME(a, b, c, ...)` | `WHERE SAME(a, b)` | `G114` |
| `PROPERTY_EXISTS(n, 'key')` | `WHERE PROPERTY_EXISTS(p, 'email')` | `G115` |
| `IN list` | `WHERE country IN ['NZ', 'AU']` | mandatory |
| `STARTS WITH`, `ENDS WITH`, `CONTAINS` | `WHERE name STARTS WITH 'A'` | mandatory |
| `EXISTS { MATCH ... }` | `WHERE EXISTS { MATCH (p)-[:KNOWS]->() }` | mandatory |

SQL-style predicate `LIKE` and `BETWEEN` syntax is not part of Selene's GQL
surface. Use `STARTS WITH`, `ENDS WITH`, or `CONTAINS` for string predicates,
and spell ranges as ordinary comparisons such as `x >= 0 AND x <= 100`.

### Expressions

Operators in precedence order (low to high): `OR`, `XOR`, `AND`, `NOT`,
predicate family (`IS ...`, `IN`, string match),
comparison (`<`, `<=`, `>`, `>=`, `=`, `<>`), concatenation (`||`),
addition (`+`, `-`), multiplication (`*`, `/`), unary (`+`, `-`),
postfix (`.prop`).

Use the ISO `MOD(x, y)` numeric function for modulus. Infix `%` and temporal
property postfix forms such as `.prop AT TIME 'ts'` are rejected at parse time.

The arithmetic and comparison operators flow through three-valued logic
when any operand is `NULL`.

### List expressions

```gql
RETURN [1, 2, 3] AS values
RETURN [1, 2] || [3, 4] AS values
RETURN CARDINALITY([1, 2, 3]) AS count
RETURN TRIM([1, 2, 3, 4], 2) AS prefix
```

Selene supports ISO list value constructors, concatenation, `CARDINALITY`, and
the ISO list `TRIM(list, count)` function. Cypher-style list subscript,
comprehension, list quantifier, and `REDUCE` expression syntax is not ISO GQL
and is rejected at parse time.

### `CASE`, `CAST`, `TRIM`, `LABELS`

```gql
RETURN
  CASE WHEN p.age < 18 THEN 'minor'
       WHEN p.age < 65 THEN 'adult'
       ELSE 'senior'
  END                                    AS bucket,
  CAST(p.age AS FLOAT)                   AS age_float,
  TRIM(LEADING ' ' FROM p.raw_name)      AS clean_name,
  LABELS(p)                              AS person_labels
```

`CASE` accepts both simple (`CASE expr WHEN val THEN result ...`) and
searched (`CASE WHEN condition THEN result ...`) forms.

### Path patterns

```gql
MATCH p = ANY SHORTEST (a:Person)-[:KNOWS *1..6]-(b:Person)
WHERE a.name = 'Ada' AND b.name = 'Lin'
RETURN p
```

`p = ...` binds the matched path to a path variable. Selectors:

| Selector | Behavior | Feature |
|---|---|---|
| (none) | All paths matching the quantified pattern. | mandatory |
| `ANY` | One arbitrary path. | `G016` |
| `ANY SHORTEST` | One arbitrary shortest path. | `G018` |
| `ALL` | All paths. | `G015` |
| `ALL SHORTEST` | All paths of minimum length. | `G017` |

Path mode modifiers (`WALK`, `ACYCLIC`, `SIMPLE`, `TRAIL`) restrict path
shape. `WALK` (the default) allows repeated nodes and edges; `TRAIL`
forbids repeated edges; `SIMPLE` forbids repeated nodes except start/end;
`ACYCLIC` forbids repeated nodes entirely.

---

## 4. Set composition

Composite queries combine two or more pipelines.

```gql
MATCH (p:Person) WHERE p.country = 'NZ' RETURN p.name
UNION
MATCH (p:Person) WHERE p.country = 'AU' RETURN p.name
```

| Operator | Status |
|---|---|
| `UNION`, `UNION ALL`, `UNION DISTINCT` | Supported (feature `GQ03`). |
| `EXCEPT`, `EXCEPT ALL`, `EXCEPT DISTINCT` | Supported (features `GQ04`, `GQ05`). |
| `INTERSECT`, `INTERSECT ALL`, `INTERSECT DISTINCT` | Supported (features `GQ06`, `GQ07`). |
| `OTHERWISE` | Supported (feature `GQ02`). |

### `LIMIT` precedence under `UNION ALL`

`LIMIT N` is a pipeline statement that attaches to the `query_pipeline`
it sits within. In a composite query (`A UNION ALL B`), `... B LIMIT N`
limits arm `B` only; arm `A` runs unlimited and the union concatenates
`A`'s rows with up to `N` rows from `B`. To limit the union total, wrap
the composite in a `CALL { ... }` table subquery and apply `LIMIT` to
the outer pipeline.

```gql
-- Per-arm: arm A runs unlimited, arm B is capped at 10
MATCH (a:Person {country: 'NZ'}) RETURN a.name
UNION ALL
MATCH (b:Person {country: 'AU'}) RETURN b.name LIMIT 10

-- Union-total: wrap the composite, LIMIT applies to the merged result
CALL {
  MATCH (a:Person {country: 'NZ'}) RETURN a.name AS name
  UNION ALL
  MATCH (b:Person {country: 'AU'}) RETURN b.name AS name
}
RETURN name LIMIT 10
```

This is a syntactic consequence of how `composite_query` parses
`set_op` between `query_pipeline` arms: a trailing `LIMIT` is absorbed
into the last arm's `pipeline_statement+` rather than the union.

### Chained `NEXT`

```gql
MATCH (p:Person {country: 'NZ'}) RETURN p
NEXT
MATCH (p)-[:KNOWS]->(q) RETURN q.name
```

`NEXT` chains two pipelines so the second pipeline observes the first
pipeline's binding table as input. Unlike `UNION` (which combines result
sets), `NEXT` is a sequential composition.

---

## 5. Aggregation

| Function | Behavior |
|---|---|
| `count(x)` | Count of non-null values of `x`. |
| `count(*)` | Count of rows. |
| `count(DISTINCT x)` | Count of distinct non-null values. |
| `sum(x)` | Numeric sum. Returns `NULL` over empty input. |
| `avg(x)` / `average(x)` | Arithmetic mean. |
| `min(x)` / `max(x)` | Order-preserving extrema, using GQL ordering rules. |
| `collect(x)` / `collect_list(x)` | List of non-null values. `COLLECT_LIST` is `GV50`. |
| `stddev_pop(x)` / `stddev_samp(x)` | Population / sample standard deviation. |

```gql
MATCH (p:Person)
RETURN p.country AS country,
       count(*) AS people,
       avg(p.age) AS mean_age,
       collect(p.name) AS names
GROUP BY p.country
ORDER BY people DESC
```

`GROUP BY` is runtime-supported as feature `GQ15`. Implicit grouping (any
non-aggregate projection in a `RETURN` that also contains an aggregate)
also works.

---

## 6. Writing data

A mutation pipeline is an optional read prologue (zero or more `MATCH` /
`FILTER` statements), one or more mutation ops, and an optional terminator
(`RETURN ...` or `FINISH`).

### `INSERT`

```gql
INSERT (a:Person {name: 'Ada', age: 36}),
       (b:Person {name: 'Lin', age: 28}),
       (a)-[:KNOWS {since: DATE '2024-01-01'}]->(b)
FINISH
```

`INSERT` accepts a path-based graph pattern. Each node pattern may declare
labels (using the `:Label` shorthand or `:LabelA & LabelB` for multiple
labels; `OR` and `NOT` are not valid in `INSERT` label sets), a property
map, and an optional variable to bind the new node. Edge patterns include
direction (`->` or `<-`), an optional variable, a label, and a property
map. Sources / targets of edge patterns can reference variables bound
earlier in the same `INSERT` or by a prior `MATCH`.

```rust
let stmt = parse(
    "INSERT (a:Person {name: 'Ada'}), (b:Person {name: 'Lin'}), (a)-[:KNOWS]->(b) FINISH"
)?;
let analyzed = analyze(stmt, &registry, None)?;
let planned = plan(&analyzed, &registry)?;
execute_statement(&planned, &mut session, &registry)?;
```

### `MERGE` (deferred)

`MERGE` is grammar-reserved, but the AST builder deliberately rejects it
with feature-not-supported status `42N01`. Current runtime support does not
include this mutation surface. Use explicit `MATCH` plus `INSERT` application
logic until a dedicated `MERGE` implementation lands.

### `SET`

`SET` accepts three item shapes in any combination, comma-separated:

```gql
MATCH (p:Person {name: 'Ada'})
SET p.age = 36,                       // property set
    p = {age: 36, country: 'NZ'},     // replace-all-properties
    p IS Senior                       // label add
```

The `set_label_item` form accepts `IS Label` or the shorthand `:Label`.

### `REMOVE`

```gql
MATCH (p:Person {name: 'Ada'})
REMOVE p.deprecated_field, p IS Legacy
```

`REMOVE` removes properties and labels. The `IS Label` / `:Label`
shorthands match `SET`.

### `DELETE` and `DETACH DELETE`

```gql
MATCH (p:Person {name: 'Ada'})
DETACH DELETE p
```

`DELETE node` fails if the node still has incident edges. `DETACH DELETE
node` removes incident edges first. `DELETE edge` always works. Multiple
targets can be deleted in one op:

```gql
MATCH (a:Person {name: 'Ada'}), (b:Person {name: 'Lin'})
DETACH DELETE a, b
```

The optional `NODETACH` keyword opts into the strict (non-detach) form
explicitly.

### Terminators

- `RETURN ...` after a mutation surfaces a binding table built from any
  surviving bindings.
- `FINISH` ends the pipeline silently (no rows surfaced). This is the
  idiomatic terminator for pure-write statements.

---

## 7. Schema (DDL)

The engine has open (schema-on-read) and closed (schema-validated) graph modes,
corresponding to GG01 and GG02. Open graphs are created from GQL through the
`selene-db` facade; closed graphs can be created through the Rust catalog API
today and from GQL once M02-PR04 part 2 lands.

### `CREATE SCHEMA` / `DROP SCHEMA`

```gql
CREATE SCHEMA /memory
CREATE SCHEMA IF NOT EXISTS /`my schema`
DROP SCHEMA IF EXISTS /memory
```

Schema references are absolute (ISO/IEC 39075:2024 §17.1): the leading `/` is
mandatory and a bare name is a syntax error (`42001`). The facade catalog has
one root directory with no child directories, so `/a/b` is an invalid reference
(`42002`). Both statements complete with the omitted-result condition `00001`;
the `IF [NOT] EXISTS` no-ops are silent and publish nothing. Dropping a schema
that still contains objects is a dependent-object error (`G1000`); dropping the
bootstrap schema `/public` is an access-rule violation (`42000`).

### `CREATE GRAPH` / `DROP GRAPH`

```gql
CREATE GRAPH /memory/episodes ANY
CREATE PROPERTY GRAPH IF NOT EXISTS scratch TYPED ANY PROPERTY GRAPH
DROP GRAPH /memory/episodes
DROP PROPERTY GRAPH IF EXISTS scratch
```

A graph reference is either absolute (`/schema/graph`) or a single name
resolved against the current working schema, which the compatibility session
fixes to `/selene/public` (§17.2 SR2a). The graph type clause is mandatory in
ISO §12.4; only the `<open graph type>` (`[TYPED | ::] ANY [[PROPERTY] GRAPH]`,
feature GG01) executes. `LIKE g` (GG04), `AS COPY OF g` (GG05), an inline
graph type (GG03), a graph type reference (GG02), `OR REPLACE`, and
`CREATE/DROP GRAPH TYPE` are rejected before planning with `42N01`; a
`NEXT`-composed catalog statement is rejected the same way.

`CREATE GRAPH` and `DROP GRAPH` complete with `00001`. `DROP GRAPH IF EXISTS`
on an absent graph completes with the warning `01G03` (§12.5 GR1). A strict
duplicate is `42N10`; a missing graph, missing schema, or wrong-kind object is
`42002`; a nonempty graph is `G1000` (all drops are RESTRICT).

A `DROP GRAPH` whose reference resolves to the protected bootstrap graph
`/selene/public/default` still performs the implementation-defined
`IM_DROP_GRAPH` factory reset of that graph instead of removing it; M02-PR05
removes this bridge together with the bootstrap catalog. Through a named
`GraphHandle`, every catalog statement is rejected with `42N01`; use the facade
`Session` or the Rust `Catalog` API.

### `CREATE NODE TYPE` / `CREATE EDGE TYPE`

```gql
CREATE NODE TYPE :Person (
    name :: STRING NOT NULL,
    age :: INTEGER,
    email :: STRING UNIQUE INDEXED,
    country :: STRING DEFAULT 'NZ'
) STRICT
```

```gql
CREATE EDGE TYPE :KNOWS (
    FROM :Person TO :Person,
    since :: DATE NOT NULL,
    weight :: FLOAT DEFAULT 1.0
) STRICT
```

Catalog DDL accepts `NOT NULL`, `DEFAULT <expr>`, `IMMUTABLE`, `UNIQUE`, and
`INDEXED [AS <name>]` property annotations. Donor full-text and time-series
constraints such as `SEARCHABLE`, `DICTIONARY`, `FILL`, `INTERVAL`, and
`ENCODING` are not part of the supported grammar and reject as syntax errors.

The trailing `STRICT` or `WARN` keyword is accepted by the grammar and stored
as `ValidationMode`. `STRICT` is the default closed-graph validation mode:
writes against a closed graph hard-fail with `G2000` when they violate the
bound graph type. `WARN` permits relaxed writes and emits warning `01N01`
(`VALIDATION_MODE_RELAXED_WRITE`) through the session warning sink after
commit. The parser observes element type names (`GG20`) and explicit key label
sets (`GG21`); neither is reported as runtime-supported while its GG02
dependency is withdrawn.

`DEFAULT <expr>` is represented in the AST and is independent from `NOT NULL`:
a property with `DEFAULT` but no `NOT NULL` remains nullable. Catalog defaults
are validated against the declared property type, stored in the graph type,
round-tripped by `SHOW NODE TYPES` / `SHOW EDGE TYPES`, recovered from durable
state, and materialized when an inserted node or edge omits the property.

`OR REPLACE` and `IF NOT EXISTS` modifiers are accepted on `CREATE NODE TYPE`
and `CREATE EDGE TYPE`. The parser observes feature `GC03`, but the generated
runtime inventory withdraws it with its GG02 dependency.

### `ALTER NODE TYPE`

```gql
ALTER NODE TYPE :Person (
    nickname :: STRING,
    metadata :: JSON DEFAULT '{}'
)
```

`ALTER NODE TYPE` is the implementation-defined `IM_ALTER_NODE_TYPE`
extension; ISO/IEC 39075:2024 defines node-type specifications but no catalog
`ALTER` statement. The operation is property-additive only. It can add nullable
properties to an existing node type in a closed graph type, while existing
nodes remain intact. A default applies to later inserts that omit the new
property; the alteration does not backfill existing rows.

New `NOT NULL` properties reject even when they declare a default. Existing
property descriptors cannot be changed, and inline `INDEXED` is unsupported on
this form; create an index separately after the alteration. Catalog commit
revalidates the live closed graph, so this rare DDL operation is not a
constant-time migration. WAL replay extends the named node type's properties in
place so the positional node-type indexes used by edge endpoints remain stable.

### `ALTER EDGE TYPE`

```gql
ALTER EDGE TYPE :CONCERNS (
    FROM :Issue, :PullRequest, :Commit TO :Document,
    commit_sha :: STRING
)
```

`ALTER EDGE TYPE` is additive only. It may widen the source and/or target
endpoint set and may add nullable properties. Existing endpoint members must
remain present; endpoint narrowing, property redefinition, and new `NOT NULL`
properties reject during catalog execution. The migration is durable catalog
state. WAL recovery applies only the changed endpoints and newly added
properties to the named edge type in place, preserving catalog declaration
order and leaving unchanged property descriptors untouched.

### `DROP NODE TYPE` / `DROP EDGE TYPE`

```gql
DROP NODE TYPE :Person IF EXISTS
DROP EDGE TYPE :KNOWS IF EXISTS
```

### `SHOW NODE TYPES` / `SHOW EDGE TYPES`

```gql
SHOW NODE TYPES
```

Returns a binding table describing the catalog's currently-declared
element types.

### Indexes (built-in procedure)

Index management goes through the `selene.create_index` and
`selene.drop_index` built-in procedures (see section 8), not through DDL
statements:

```gql
CALL selene.create_index('idx_person_email', ':Person', 'email')
```

A grammar form `CREATE INDEX ... ON :Label (col)` parses but is not the
canonical surface; index DDL routes the same audit events whether
emitted from the CALL or the DDL form.

---

## 8. `CALL` and procedures

Procedure calls invoke named functions registered in the native procedure
registry, or execute an inline query subquery with `CALL { ... }`. A
named `CALL` accepts positional arguments and yields a tabular result via
`YIELD`. `OPTIONAL CALL` preserves each input row when the call result is
empty, filling yielded columns with `NULL`. There is no procedure-pack or
loadable-extension machinery behind the native registry.

```gql
CALL algo.pagerank('person_graph', 0.85, 30)
YIELD node_id, score
FILTER score > 0.01
RETURN node_id, score
ORDER BY score DESC
LIMIT 20
```

Form:

```text
[ OPTIONAL ] CALL <namespace>.<procedure>(args) [ YIELD col1 [, col2 ...] ]
[ OPTIONAL ] CALL [ (var1 [, var2 ...]) ] { <query pipeline> } [ YIELD col1 [, col2 ...] ]
```

`YIELD *` yields every output column. Each yield column can be aliased
(`YIELD col AS alias`). Use a following pipeline `FILTER` statement to filter
procedure output.

### Built-in `selene.*` procedures

| Procedure | Tier | Purpose |
|---|---|---|
| `selene.health` | Graph | Basic graph health counters. |
| `selene.feature_status` | Graph | Surfaces generated capability status, profile relation, claim/evidence summary, and profile hash. |
| `selene.verify` | Graph | Integrity check over graph invariants. |
| `selene.compaction_stats` | Graph | Graph row compaction pressure counters. |
| `selene.create_index`, `selene.drop_index` | Mutation | Create or drop scalar property indexes through the mutation funnel. |
| `selene.create_vector_index`, `selene.drop_vector_index` | Mutation | Register or drop vector indexes over `(label, property)`. |
| `selene.vector_search_*`, `selene.vector_score_*` | Graph | Exact, ANN, node/edge-filtered, candidate-scoped, neighbor, expanded-candidate, and batched vector retrieval. |
| `selene.vector_candidate_states` | Graph | Discover maintained graph-derived candidate states. |
| `selene.reachable_nodes` | Graph | Bounded transitive graph reachability candidate production from explicit root nodes. |
| `selene.vector_index_stats` | Graph | Vector index memory and cardinality statistics. |
| `selene.rebuild_vector_indexes`, `selene.rebuild_recommended_vector_indexes` | Maintenance | Rebuild derived in-memory vector index state from primary graph values. |
| `selene.create_text_index`, `selene.drop_text_index` | Mutation | Register or drop maintained BM25 text indexes. |
| `selene.text_index_stats` | Graph | Text index memory and cardinality statistics. |
| `selene.property_index_stats` | Graph | Property index drift and cardinality statistics; `answers_probes` is false while an index is demoted to a scan. |
| `selene.text_search_nodes`, `selene.text_score_nodes`, `selene.text_score_nodes_batch`, `selene.text_score_candidate_state_expanded_batch` | Graph | Exact BM25 search plus node/edge-filtered and candidate-scoped text scoring. |
| `selene.reciprocal_rank_fusion` | Graph | Fuse ranked node lists with Reciprocal Rank Fusion. |
| `selene.json_contains_nodes`, `selene.json_path_*_nodes` | Graph | Exact JSON containment, path-existence, path-containment, and path-value search over node properties. |
| `selene.json_contains_candidate_nodes`, `selene.json_path_*_candidate_nodes` | Graph | Candidate-scoped JSON filters over explicit `LIST<NODE>` inputs. |
| `selene.compact` | Maintenance | Compact dead graph rows out of the live store. |

The 50 platform built-ins are registered by the native
`selene-gql` `BuiltinProcedureRegistry` (the sole frozen production
`ProcedureRegistry` impl) and documented in its rustdoc.

Global text search and ANN vector search accept optional indexed node-property
filters and indexed edge-property filters. Edge filters use
`edge_filter_label`, `edge_filter_property`, `edge_filter_values`, and
`edge_filter_endpoint`; endpoint values admit the matching edge `source`,
`target`, or `both` endpoints as candidate nodes. Pass `NULL, NULL` for
`filter_property` and `filter_values` when using an edge-only filter.

### Algorithm procedures (`algo.*`)

Bound natively over the mandatory `selene-algorithms` crate by the same
`BuiltinProcedureRegistry`. The 19 procedure names are:

```text
algo.projection_build, algo.projection_get, algo.projection_drop, algo.projection_list,
algo.pagerank, algo.betweenness, algo.label_propagation, algo.louvain,
algo.triangle_count, algo.wcc, algo.scc, algo.wcc_count, algo.scc_count,
algo.topological_sort, algo.articulation_points, algo.bridges,
algo.dijkstra, algo.sssp, algo.apsp
```

See [`graph-algorithms.md`](graph-algorithms.md) for argument shapes and
result columns. `algo.pagerank` can also filter result candidates through the
same indexed edge-property filter group used by global retrieval procedures:
`edge_filter_label`, `edge_filter_property`, `edge_filter_values`, and
`edge_filter_endpoint`.

### Registry construction

`EmptyProcedureRegistry` is the no-op registry used by the README example.
A real embedder constructs the native `BuiltinProcedureRegistry`, which is
frozen at construction: it allocates a fixed set of handles for the 50
platform built-ins plus 19 `algo.*` procedures and never changes thereafter
(`registry_version()` is a constant `0`). It can be shared across threads
via `Arc`. There are no loadable third-party packs to register.

---

## 9. Transaction control

selene-db's default isolation is **serializable** (clause 4.6); the engine
uses strict-serializable under a single write lock per graph with
lock-free reads. Generated Annex B records `IE002` and `IE004` settle this;
`selene_profile::annex_b_by_id` provides direct lookup.

Statements outside an explicit transaction auto-commit at statement end
(implementation-defined choice `IE001`).

```gql
START TRANSACTION
```

```gql
COMMIT
```

```gql
ROLLBACK
```

Inside an explicit transaction, multiple statements share one snapshot
and one write boundary. A failed statement marks the transaction aborted;
subsequent statements (other than `ROLLBACK`) return
`ExecutorError::InFailedTransaction`.

```rust
let mut session = selene_gql::Session::new(&graph);

execute_statement(&plan_for("START TRANSACTION"), &mut session, &registry)?;
execute_statement(&plan_for("INSERT (:Person {name: 'Ada'}) FINISH"), &mut session, &registry)?;
execute_statement(&plan_for("INSERT (:Person {name: 'Lin'}) FINISH"), &mut session, &registry)?;
execute_statement(&plan_for("COMMIT"), &mut session, &registry)?;
```

Mixed catalog-and-data transactions are forbidden (implementation-defined
choices `IE006`, `IE007`): a transaction may either modify schema or
modify data, but not both. Feature `GP18` (mixed catalog/data) is not
present in the current register.

Multi-graph transactions (`GT03`) are not runtime-supported; one transaction touches
exactly one graph.

---

## 10. GQL Flagger

The Flagger looks up each parser-observed feature's generated admission
disposition. A record is accepted if and only if it is a direct ISO selection
or is runtime-supported. In the current inventory, rejected records are
implied-only or unselected and are unsupported or referenced. Rejections carry
the canonical ID, name, source span, and non-support rationale. Parser
admission, runtime status, formal claim state, and evidence are independent
generated fields. Rejection happens **before** execution.

Examples of rejected constructs:

| Construct | Reason | Failure mode |
|---|---|---|
| `CREATE PROCEDURE pkg.fn() { LET x = 1 RETURN x }` | Procedure-local definitions (`GP05`-`GP13`) are deferred. | Parser error. |
| `CALL pkg.fn(TABLE rows)` | Binding tables as procedure arguments (`GP14`) are deferred. | Parser error. |
| `CALL pkg.fn(GRAPH g)` | Graphs as procedure arguments (`GP15`) are deferred. | Parser error. |
| `CREATE GRAPH demo` | ISO §12.4 requires an `<open graph type>` or `<of graph type>` clause. | Parser error (`42001`). |
| `CREATE GRAPH demo LIKE other` | Graph type like a graph (`GG04`) is not implemented. | Feature error for `GG04`. |
| `MERGE (n:Person {id: 1})` | `MERGE` mutation lowering is deferred. | Parser error. |
| `RETURN NULL IS TYPED GRAPH AS ok` | Graph reference value types (`GV60`) are deferred. | Flagger error. |
| `CAST(x AS FLOAT16)` | Feature `GV20` is unsupported. | Flagger error. |
| `CAST(x AS FLOAT128)` | Feature `GV25` is unsupported. | Flagger error. |
| Cypher-only `CREATE (n:Foo)-[:R]->(m:Bar)` (without the `INSERT` keyword) | Not ISO GQL surface. | Parser error. |
| Cypher-only `WHERE n.x =~ '.*foo.*'` (regex match) | Not ISO GQL surface. | Parser error. |

### Runtime feature introspection

`CALL selene.feature_status()` preserves `feature_id`, `status`, and `rationale`
as its first three columns, followed by `feature_name`, `surface`,
`profile_relation`, `claim_state`, `evidence_status`, `evidence_count`, and
`profile_hash`. Rows come directly from `selene_profile::capabilities()` in
generated runtime order. Evidence counts summarize registered references and do
not expose internal paths. This inventory is not a release conformance claim.

---

## 11. Error categories

selene-db separates errors by phase. Each phase has its own error enum
with `miette::Diagnostic` derives and `GQLSTATUS`-aligned codes.

| Phase | Error type | What it means |
|---|---|---|
| Parser | `selene_gql::ParserError` | Syntactic error or Flagger rejection during parse. Carries source spans suitable for `miette` rendering. |
| Analyzer | `selene_gql::AnalysisError` | Scope, type, and write-set rejection during analysis. Reports unresolved variables, type mismatches, and mutation write-set conflicts. |
| Planner | `selene_gql::PlannerError` | Lowering failure. Reports missing procedure signatures, undeclared indexes, or unrepresentable plan shapes. |
| Executor | `selene_gql::ExecutorError` | Runtime failure. Reports graph-mutation rejection (`GraphMutation`), failed-transaction reentry (`InFailedTransaction`), procedure errors (`ProcedureError`), implementation-defined surfaces (`ImplementationDefined`), and Boolean / value-type runtime errors. |

Errors that reach the embedder are owned types, never panics. The
diagnostic codes follow the GQLSTATUS table in
`selene-core::gqlstatus::ALL_GQLSTATUS_NAMES`.

---

## 12. What's NOT supported

The current c5 surface is deliberately narrow. The list below names what is
explicitly absent. `selene_profile::capability` returns the canonical status and
non-support rationale for each feature ID.

| Surface | Status |
|---|---|
| Cypher grammar | Not supported. Use ISO GQL syntax. |
| SQL grammar | Not supported. |
| SPARQL grammar | Not supported. |
| Procedure-local definitions (`CREATE PROCEDURE { ... }`) | Unsupported/deferred (features `GP05`-`GP13`). Inline query subqueries (`GP01`-`GP03`) and named procedure calls (`GP04`) are runtime-supported. |
| Binding tables or graphs as procedure arguments | Unsupported/deferred (features `GP14`, `GP15`). |
| Procedure-local variables | Unsupported/deferred (features `GP05`-`GP15`). |
| Mixed catalog/data transactions | Unsupported (feature `GP18`). |
| Multi-graph transactions | Unsupported (feature `GT03`). |
| Graph / table reference type spellings (`GRAPH`, `TABLE` as types) | Outside current runtime support (features `GV60`-`GV61`). |
| `FLOAT16`, `FLOAT128`, `FLOAT256` | Unsupported (`GV20`, `GV25`, `GV26`). `REAL` / `DOUBLE` synonyms are runtime-supported. |
| 256-bit integers (`INT256`, `UINT256`) | Unsupported. |
| Time-series query syntax | Out of scope. Future first-party extension allocation `TIMS`. |
| RDF / SPARQL bridge syntax | Out of scope. Future first-party extension allocation `GRPR`. |
| Recursive CTEs (`WITH RECURSIVE`) | Not in ISO GQL; not supported. |
| Broader `ALTER NODE TYPE` migrations | Property rename/removal/retyping, validation-mode changes, key-label changes, and inline indexes are not supported. Add nullable properties only, then create indexes separately. |
| Wire format | Out of scope (ISO GQL Clause 4.2.3). Embedders pick their own transport. |

Where the AST has a node for a construct but the analyzer rejects it, the
node exists to keep the parser total and the diagnostic precise.
