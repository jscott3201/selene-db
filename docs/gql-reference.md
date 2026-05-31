# GQL reference

This document is the query-author's reference for the GQL surface that
selene-db exposes. It assumes you have read the README quickstart and can
build a `SharedGraph` plus an `EmptyProcedureRegistry`.

selene-db targets **ISO/IEC 39075:2024 minimum conformance** plus a curated
subset of optional features. The parser is **strict ISO GQL**: no Cypher,
no SQL, no SPARQL grammar. Constructs outside the v1.0 claimed feature
register are rejected at parse time by the GQL Flagger (ISO GQL Clause 24.6).

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

## 1. What's supported

The supported feature set is declared in
`selene-core::feature_register::SUPPORTED_FEATURES` and rendered for the
spec docs by the build. The table below summarizes the major clause groups.

| Group | Coverage | Notes |
|---|---|---|
| Read query (`MATCH`, `OPTIONAL MATCH`, `WHERE`, `RETURN`, `WITH`, `UNWIND`, `ORDER BY`, `LIMIT`, `OFFSET`, `DISTINCT`) | Full | The pipeline form is canonical; `SELECT ... FROM` desugars at the AST level. |
| Set composition (`UNION`, chained `NEXT`) | Partial | Only `UNION` / `UNION ALL` / `UNION DISTINCT` are supported (feature `GQ03`). `EXCEPT`, `INTERSECT`, and `OTHERWISE` parse but the analyzer rejects them. |
| Aggregation (`count`, `sum`, `avg`, `min`, `max`, `collect`, `stddev_pop`, `stddev_samp`) | Full | `GROUP BY` is feature `GQ15` and is claimed. |
| Mutation (`INSERT`, `MERGE`, `SET`, `REMOVE`, `DELETE`, `DETACH DELETE`) | Full | `MutationPipeline` accepts an optional terminator (`RETURN` or `FINISH`). |
| DDL (`CREATE/DROP GRAPH`, `CREATE/DROP NODE TYPE`, `CREATE/DROP EDGE TYPE`, `SHOW NODE TYPES`, `SHOW EDGE TYPES`) | Full | Graph types claim features `GG01` (open) and `GG02` (closed); explicit element type names and key label sets are `GG20` / `GG21`. |
| Procedure calls (`CALL ns.proc(args) YIELD col1, col2`) | Full | Named procedure call is feature `GP04`. Inline procedures (`GP01`-`GP15`) are not claimed in v1.0. |
| Transaction control (`START TRANSACTION`, `COMMIT`, `ROLLBACK`) | Full | Feature `GT01`. Multi-graph transactions (`GT03`) are not claimed. |
| Path patterns (variable-length, ANY/ALL SHORTEST) | Partial | `ANY`, `ANY SHORTEST`, `ALL`, `ALL SHORTEST` selectors are claimed (`G015`-`G018`). Counted shortest selectors (`G019`, `G020`) are not. |
| Predicates (`IS DIRECTED`, `IS LABELED`, `IS SOURCE/DESTINATION OF`, `ALL_DIFFERENT`, `SAME`, `PROPERTY_EXISTS`) | Full | Features `G110`-`G115`. |

Statements outside the claimed feature set fail with a Flagger error during
parsing or analysis, never at runtime.

---

## 2. Data types

### Mandatory types (ISO minimum conformance)

| GQL type | Literal syntax | `Value` variant | Notes |
|:---|:---|:---|:---|
| `BOOLEAN` | `TRUE`, `FALSE`, `UNKNOWN` | `Value::Bool` | Three-valued logic applies to `=`, `<>`, comparisons, and Boolean composition. |
| `INTEGER` / `INT` | `42`, `-17`, `0`, `1_000` (underscores allowed) | `Value::Int` (i64) | Default integer is i64. Implementation-defined IA037 / ID028 set i64 default with i128 promotion when context demands. |
| `FLOAT` | `3.14`, `-0.5`, `1.0e6`, `2.5e-3` | `Value::Float` (f64) | IEEE 754 binary64 (feature `GA01`). |
| `STRING` | `'single quotes only'`, `'it''s ok'`, `'\n'` escapes | `Value::String(IStr)` | Single quotes only — double quotes are delimited identifiers. `''` and backslash escapes are honored. |

### Optional types claimed in v1.0

| GQL type | Literal syntax | `Value` variant | Feature |
|:---|:---|:---|:---|
| `INT8`, `INT16`, `INT32` | `CAST(x AS INT32)` | `Value::Int` | `GV02`, `GV04`, `GV07` |
| `UINT8`, `UINT16`, `UINT32`, `UINT64` | `42u`, `CAST(x AS UINT32)` | `Value::Uint` | `GV01`, `GV03`, `GV06`, `GV11` |
| `INT128`, `UINT128` | hex/oct/bin literals or `CAST` | `Value::Int128`, `Value::Uint128` | `GV13`, `GV14` |
| `DECIMAL` | `CAST('1.23' AS DECIMAL)` | `Value::Decimal` (`rust_decimal::Decimal`) | `GV17`, 28 significant digits |
| `FLOAT32` | `1.5f` | `Value::Float32` | `GV21` |
| `BYTES` | `CAST(x AS BYTES)` | `Value::Bytes` | `GV35` |
| `DATE` | `DATE '2026-05-16'` | `Value::Date` | `GV39` |
| `LOCAL DATETIME` | `LOCAL DATETIME '2026-05-16T08:30:00'` | `Value::LocalDateTime` | `GV39` |
| `LOCAL TIME` | `LOCAL TIME '08:30:00'` | `Value::LocalTime` | `GV39` |
| `ZONED DATETIME` | `ZONED DATETIME '2026-05-16T08:30:00-07:00'` | `Value::ZonedDateTime` | `GV40` |
| `ZONED TIME` | `ZONED TIME '08:30:00-07:00'` | `Value::ZonedTime` | `GV40` |
| `DURATION` | `DURATION 'PT1H30M'` or `DURATION('1h30m')` | `Value::Duration` | `GV41` |
| `LIST<T>` | `[1, 2, 3]`, `CAST(x AS LIST<INTEGER>)` | `Value::List` | `GV50` |
| `PATH` | constructed by `MATCH` path variables | `Value::Path` | `GV55` |

`NULL` and `UNKNOWN` are first-class. `NULL` represents missing data;
`UNKNOWN` is the three-valued-logic Boolean. Three-valued logic flows
through every Boolean operator (`AND`, `OR`, `XOR`, `NOT`) and through
the comparison family.

### Optional types deliberately not claimed in v1.0

`RECORD` (open / closed / nested, `GV45`-`GV48`), graph and binding-table
reference types (`GV60`-`GV61`), explicit value-type nullability syntax
(`GV90`), `FLOAT16` / `FLOAT128` / `FLOAT256`, 256-bit integers, and the
`REAL` synonym all carry rationale entries in
`feature_register::NOT_SUPPORTED_RATIONALE`. Query that mentions one of
these types is rejected at parse or analyze time.

### Numeric literal forms

| Form | Example | Notes |
|---|---|---|
| Decimal integer | `1234`, `-1234`, `1_000_000` | i64 unless explicit cast. |
| Unsigned suffix | `42u` | Parses to `Value::Uint`. |
| Hex | `0xCAFE`, `-0x1A` | Feature `GL01` (selene extension). |
| Oct | `0o777` | Feature `GL02`. |
| Bin | `0b1010` | Feature `GL03`. |
| Float | `1.5`, `1.5e10`, `1.5f` (f32), `1.5d` (f64 explicit) | Default f64. |

### Identifiers and delimited identifiers

| Form | Example | Notes |
|---|---|---|
| Unquoted | `name`, `Person`, `customer_id` | Unicode letters / digits / underscore. Reserved keywords cannot appear unquoted. |
| Double-quoted | `"first name"`, `"with ""quote"""` | Spec form. `""` escapes a literal double quote. |
| Backtick-quoted | `` `first name` `` | selene extension; semantically identical to double-quoted. |
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
in-scope binding. `RETURN NO BINDINGS` produces an empty binding table
(useful for write pipelines that don't need to surface rows).

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

### `UNWIND`

```gql
UNWIND [1, 2, 3, 4] AS x
RETURN x * x AS squared
```

`UNWIND` flattens a list expression into row-per-element. The expression
can be a list literal, a list-typed property, or any expression evaluating
to `LIST<T>`.

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
expression (similar to `UNWIND` but as a pipeline op).

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
| `BETWEEN a AND b` | `WHERE x BETWEEN 0 AND 100` | mandatory |
| `IN list` | `WHERE country IN ['NZ', 'AU']` | mandatory |
| `LIKE pattern` | `WHERE name LIKE 'A%'` | mandatory |
| `STARTS WITH`, `ENDS WITH`, `CONTAINS` | `WHERE name STARTS WITH 'A'` | mandatory |
| `EXISTS { MATCH ... }` | `WHERE EXISTS { MATCH (p)-[:KNOWS]->() }` | mandatory |
| `COUNT { MATCH ... }` | `RETURN COUNT { MATCH (p)-[:KNOWS]->() }` | mandatory |

### Expressions

Operators in precedence order (low to high): `OR`, `XOR`, `AND`, `NOT`,
predicate family (`IS ...`, `IN`, `LIKE`, `BETWEEN`, string match),
comparison (`<`, `<=`, `>`, `>=`, `=`, `<>`), concatenation (`||`),
addition (`+`, `-`), multiplication (`*`, `/`, `%`), unary (`+`, `-`),
postfix (`.prop`, `[index]`, `.prop AT TIME 'ts'`).

The arithmetic and comparison operators flow through three-valued logic
when any operand is `NULL`.

### List expressions

```gql
RETURN [x IN [1, 2, 3, 4] WHERE x > 2 | x * x]              // list comprehension
RETURN ALL(x IN range WHERE x > 0)                           // universal quantifier
RETURN ANY(x IN range WHERE x = target)                      // existential quantifier
RETURN NONE(x IN range WHERE x = forbidden)
RETURN SINGLE(x IN range WHERE x = unique_target)
RETURN REDUCE(acc = 0, x IN [1, 2, 3] | acc + x)            // fold
```

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

| Operator | v1.0 status |
|---|---|
| `UNION`, `UNION ALL`, `UNION DISTINCT` | Supported (feature `GQ03`). |
| `EXCEPT`, `EXCEPT ALL`, `EXCEPT DISTINCT` | Parses; analyzer rejects (features `GQ04`, `GQ05` not claimed). |
| `INTERSECT`, `INTERSECT ALL`, `INTERSECT DISTINCT` | Parses; analyzer rejects (`GQ06`, `GQ07` not claimed). |
| `OTHERWISE` | Parses; analyzer rejects (`GQ09` not claimed). |

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

`GROUP BY` is feature `GQ15` and is claimed. Implicit grouping (any
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

### `MERGE`

```gql
MERGE (p:Person {email: 'ada@example.org'})
ON CREATE SET p.created = LOCAL DATETIME '2026-05-16T00:00:00'
ON MATCH SET p.last_seen = LOCAL DATETIME '2026-05-16T00:00:00'
```

`MERGE` matches the pattern; if no row matches, the pattern is created.
`ON CREATE SET` runs for newly-created rows; `ON MATCH SET` runs for
existing rows. Either clause is optional. Repetition of the same clause
is rejected at parse time.

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

selene-db supports two graph types: GG01 (open, schema-on-read) and GG02
(closed, schema-validated). The default is open; closed graphs are
opt-in.

### `CREATE GRAPH` / `DROP GRAPH`

```gql
CREATE GRAPH analytics IF NOT EXISTS
```

```gql
DROP GRAPH analytics IF EXISTS
```

`CREATE GRAPH` and `DROP GRAPH` accept the `IF [NOT] EXISTS` modifier
(feature `GC05`). `CREATE OR REPLACE GRAPH` replaces an existing graph
atomically. Graph management is feature `GC04`.

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

Property constraints recognized by the AST (`TypePropertyConstraint`):
`NOT NULL`, `DEFAULT <expr>`, `IMMUTABLE`, `UNIQUE`, `INDEXED`,
`SEARCHABLE`, `DICTIONARY`, `FILL <name>`, `INTERVAL '<duration>'`,
`ENCODING <name>`.

The trailing `STRICT` or `WARN` keyword is accepted by the grammar and stored
as `ValidationMode`, but v1.1 does not enforce validation-mode semantics at
runtime. Catalog DDL carrying `STRICT` or `WARN` is currently rejected with
GQLSTATUS `5GQL0`. Closed-graph type validation is separate from
`ValidationMode`: writes against a closed graph hard-fail with `G2000` when
they violate the bound graph type. Element type names (`GG20`) and explicit
key label sets (`GG21`) are claimed.

`DEFAULT <expr>` is represented in the AST and is independent from `NOT NULL`:
a property with `DEFAULT` but no `NOT NULL` remains nullable. Runtime DEFAULT
application is not implemented in v1.1, and catalog DDL containing `DEFAULT`
is rejected with `5GQL0`.

`OR REPLACE` and `IF NOT EXISTS` modifiers are accepted on `CREATE NODE
TYPE` and `CREATE EDGE TYPE` (feature `GC03`).

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
registry. A `CALL` accepts positional arguments and yields a tabular
result via `YIELD`. The `CALL` grammar is unchanged ISO GQL (external
procedures per ISO `IW010`); there is no procedure-pack or loadable-extension
machinery behind it.

```gql
CALL algo.pagerank('person_graph', 0.85, 30)
YIELD node_id, score
WHERE score > 0.01
RETURN node_id, score
ORDER BY score DESC
LIMIT 20
```

Form:

```text
CALL <namespace>.<procedure>(args) [ YIELD col1 [, col2 ...] [ WHERE expr ] ]
```

`YIELD *` yields every output column. Each yield column can be aliased
(`YIELD col AS alias`). The optional inline `WHERE` filters the call's
output before it joins the surrounding pipeline.

### Built-in `selene.*` procedures

| Procedure | Tier | Purpose |
|---|---|---|
| `selene.health` | Graph | Liveness/readiness probe. Returns engine version, snapshot count, WAL state. |
| `selene.feature_status` | Graph | Surfaces the claimed ISO feature register at runtime. |
| `selene.verify('deep')` | Graph | Integrity check over the live graph. The optional `deep` flag defaults to `false`. |
| `selene.create_index('name', ':Label', 'prop')` | Mutation | Create a secondary index. Audits through the mutation funnel. |
| `selene.drop_index('name')` | Mutation | Drop a secondary index. Audits through the mutation funnel. |

The 5 platform built-ins are registered by the native
`selene-gql` `BuiltinProcedureRegistry` (the sole frozen production
`ProcedureRegistry` impl) and documented in its rustdoc.

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
result columns.

### Registry construction

`EmptyProcedureRegistry` is the no-op registry used by the README example.
A real embedder constructs the native `BuiltinProcedureRegistry`, which is
frozen at construction (D16): it allocates a fixed set of handles for the 5
platform built-ins + 19 `algo.*` procedures and never changes thereafter
(`registry_version()` is a constant `0`). It can be shared across threads
via `Arc`. There are no loadable third-party packs to register.

---

## 9. Transaction control

selene-db's default isolation is **serializable** (clause 4.6); the engine
uses strict-serializable under a single write lock per graph with
lock-free reads. Implementation-defined choices `IE002` and `IE004` settle
this in `feature_register::ANNEX_B_REGISTER`.

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
claimed in v1.0.

Multi-graph transactions (`GT03`) are not claimed; one transaction touches
exactly one graph.

---

## 10. GQL Flagger

The Flagger (ISO GQL Clause 24.6) rejects constructs outside the v1.0
claimed feature register at parse or analysis time. Rejection happens
**before** execution; there is no runtime "unsupported feature" surprise.

Examples of rejected constructs:

| Construct | Reason | Failure mode |
|---|---|---|
| `MATCH (p) RETURN ALL SHORTEST 3 PATH ...` | Feature `G019` (counted shortest) not claimed. | Flagger error. |
| `CALL { CREATE PROCEDURE ... }` (inline procedure body) | Features `GP01`-`GP15` not claimed. | Flagger error. |
| `MATCH ... RETURN ... EXCEPT MATCH ...` | Feature `GQ04` not claimed. | Flagger error. |
| `RECORD<a INTEGER, b STRING>` in a type position | Features `GV45`-`GV48` not claimed. | Flagger error. |
| `CAST(x AS FLOAT16)` | Feature `GV20` not claimed. | Flagger error. |
| Cypher-only `CREATE (n:Foo)-[:R]->(m:Bar)` (without the `INSERT` keyword) | Not ISO GQL surface. | Parser error. |
| Cypher-only `WHERE n.x =~ '.*foo.*'` (regex match) | Not ISO GQL surface. | Parser error. |

### Runtime feature introspection

A future `CALL selene.feature_status` procedure may surface the claimed
feature register at runtime. The canonical source today is the
`feature_register` module in `selene-core`:
`SUPPORTED_FEATURES`, `NOT_SUPPORTED_RATIONALE`, and `is_supported`.

---

## 11. Error categories

selene-db separates errors by phase. Each phase has its own error enum
with `miette::Diagnostic` derives and `GQLSTATUS`-aligned codes.

| Phase | Error type | What it means |
|---|---|---|
| Parser | `selene_gql::ParserError` | Syntactic error or Flagger rejection during parse. Carries source spans suitable for `miette` rendering. |
| Analyzer | `selene_gql::AnalysisError` | Scope / type / write-set / Flagger rejection during analysis. Reports unresolved variables, type mismatches, mutation write-set conflicts, and unclaimed features. |
| Planner | `selene_gql::PlannerError` | Lowering failure. Reports missing procedure signatures, undeclared indexes, or unrepresentable plan shapes. |
| Executor | `selene_gql::ExecutorError` | Runtime failure. Reports graph-mutation rejection (`GraphMutation`), failed-transaction reentry (`InFailedTransaction`), procedure errors (`ProcedureError`), implementation-defined surfaces (`ImplementationDefined`), and Boolean / value-type runtime errors. |

Errors that reach the embedder are owned types, never panics. The
diagnostic codes follow the GQLSTATUS table in
`selene-core::gqlstatus::ALL_GQLSTATUS_NAMES`.

---

## 12. What's NOT supported

The v1.0 surface is deliberately narrow. The list below names what is
explicitly absent. The canonical rationale is
`feature_register::NOT_SUPPORTED_RATIONALE`.

| Surface | Status |
|---|---|
| Cypher grammar | Not supported. Use ISO GQL syntax. |
| SQL grammar | Not supported. |
| SPARQL grammar | Not supported. |
| `EXCEPT`, `INTERSECT`, `OTHERWISE` set operators | Parses; analyzer rejects (features `GQ04`-`GQ09`). |
| Counted shortest path (`G019`, `G020`) | Not claimed. Use `ANY SHORTEST` or `ALL SHORTEST`. |
| Inline procedure definitions (`CREATE PROCEDURE { ... }`) | Not claimed (features `GP01`-`GP03`, `GP05`-`GP15`). Named procedures (`GP04`) are served only by the native built-in registry. |
| Procedure-local variables | Not claimed (features `GP05`-`GP15`). |
| Mixed catalog/data transactions | Not claimed (feature `GP18`). |
| Multi-graph transactions | Not claimed (feature `GT03`). |
| `RECORD` type expressions | Not claimed (features `GV45`-`GV48`). The `Value::Record` variant exists in core but is not exposed in the type grammar. |
| Graph / table reference type spellings (`GRAPH`, `TABLE` as types) | Not claimed (features `GV60`-`GV61`). |
| Explicit value-type nullability syntax (`STRING NOT NULL` in type expressions) | Not claimed (feature `GV90`). The DDL `NOT NULL` property constraint is supported separately. |
| `FLOAT16`, `FLOAT128`, `FLOAT256`, `REAL` synonym | Not claimed. |
| 256-bit integers (`INT256`, `UINT256`) | Not claimed. |
| Time-series query syntax | Out of scope. Future first-party extension allocation `TIMS`. |
| RDF / SPARQL bridge syntax | Out of scope. Future first-party extension allocation `GRPR`. |
| Recursive CTEs (`WITH RECURSIVE`) | Not in ISO GQL; not supported. |
| Wire format | Out of scope (ISO GQL Clause 4.2.3). Embedders pick their own transport. |

Where the AST has a node for a construct but the analyzer rejects it, the
node exists to keep the parser total and the diagnostic precise.
