# Changelog

All notable changes to selene-db are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Chore

- Local test invocation aligned with CI (nextest + line-tables-only debug +
  `.config/nextest.toml`). See CLAUDE.md Build & test.

### Added

- Implementation-defined named index DDL in `selene-gql`: `CREATE INDEX <name>
  ON :Label(property)` and `DROP INDEX <name>` now execute for single-property
  node indexes, infer the storage index kind from the declared property type,
  resolve drops by catalog name, enforce DDL-level name uniqueness, and record
  the vendor `IM_INDEX_DDL` feature ID. Composite-property indexes, edge-property
  indexes, and `SHOW INDEXES` provider aggregation remain split into BRIEF-140b
  through BRIEF-140d.
- Implementation-defined `EXTENDS` property composition for `CREATE NODE TYPE`
  and `CREATE EDGE TYPE` in `selene-gql`. Parent properties are flattened at
  CREATE time, same-kind parents are required, exact-match redeclarations
  succeed without duplication, mismatches raise `GraphTypeViolation`, and
  feature attribution records the vendor `IM_EXTENDS` feature ID.
- ISO/IEC 39075:2024 cluster-R string and aggregate conformance in
  `selene-gql`: §19.7 `IS [NOT] NORMALIZED`, §20.24 `NORMALIZE`, `LEFT`,
  `RIGHT`, multi-character TRIM family (GF05), explicit TRIM syntax (GF06),
  GF10 attribution for ISO `STDDEV_POP` / `STDDEV_SAMP` / `COLLECT_LIST`, and
  GF11 `PERCENTILE_CONT` / `PERCENTILE_DISC` binary aggregates. New substring
  and trim data exceptions emit ISO Table 8 `22011` and `22027`; string
  results preserve the `ExternalString` DoS invariant. SQL-only drift
  (`REPLACE`, `LPAD`, `RPAD`, `POSITION`, `SIGN`, `RAND`) remains out of
  scope.
- Implementation-defined first-class UUID support in `selene-gql`: `UUID`
  type names and literals, `uuid_v4()`, `uuid_v7()`, `uuid('<string>')`,
  `CAST(... AS UUID)`, UUID typed-index routing, and `CAST(UUID AS STRING)`
  rendering through `ExternalString`. Feature attribution records the vendor
  `IM_UUID` feature ID.
- ISO/IEC 39075:2024 §20.10 `ELEMENT_ID` (G100), §20.22 `CARDINALITY`
  (GF12), and minimum-conformance `CHAR_LENGTH` / `CHARACTER_LENGTH` scalar
  functions in `selene-gql`. `ELEMENT_ID` returns `ExternalString` node/edge
  IDs; `CARDINALITY` covers binding-table references, paths, lists, and
  records; `CHAR_LENGTH` aliases the existing Unicode scalar-counting
  `LENGTH` implementation. Session parameters now support table bindings
  through `Session::bind_table_parameter(...)` for binding-table-reference
  tests and procedure output handoff.
- ISO/IEC 39075:2024 §20.22 numeric value function clusters in `selene-gql`:
  GF01 enhanced numeric feature registration for `ABS`, `MOD`, `FLOOR`,
  `CEIL`/`CEILING`, and `SQRT`; GF02 trigonometric functions (`SIN`, `COS`,
  `TAN`, `COT`, `SINH`, `COSH`, `TANH`, `ASIN`, `ACOS`, `ATAN`, `DEGREES`,
  `RADIANS`); and GF03 logarithmic functions (`LN`, `LOG`, `LOG10`, `EXP`,
  plus `POWER` feature claiming). `LN(<=0)` now emits GQLSTATUS `2201E`
  invalid-argument-for-natural-logarithm.
- Explicit `CAST(<expr> AS <type>)` expressions in `selene-gql` per ISO/IEC
  39075:2024 §22. New `ValueExpr::Cast { value, target_type, span }` AST
  variant; feature `GE08` (CAST operator) is registered as supported. The
  runtime dispatch matrix covers numeric ↔ numeric (Integer/Float), string
  ↔ numeric (strict parse), boolean ↔ string (lowercase only), boolean ↔
  integer (0/1), and `LIST<T>` element-wise casts. NULL → ANY returns NULL
  per the §22 universal rule. Failure modes emit `22018`
  (invalid-character-value-for-cast — new in the Table 8 map), `22003`
  (numeric overflow), `22000` (boolean domain), or `42N01` for
  intentionally unsupported source/target combinations (NODE / EDGE / PATH
  / RECORD sources; `NULL` / `NOTHING` targets). The analyzer's
  `bind_value_expr` recursive walker now grows the stack via
  `stacker::maybe_grow(64K, 1MB)` so future `ValueExpr` variant additions
  cannot reach into the 2 MB default macOS pthread stack at the depth-256
  contract enforced by `check_expr_depth`.
- `EdgeEndpointDef::OneOf` for polymorphic-enumerated edge endpoints in
  `selene-graph` (storage `Vec<u32>`) and `selene-core` (WAL
  `SmallVec<[NodeTypeRef; 4]>`). Catalog DDL of the form
  `CREATE EDGE TYPE :E (FROM :A, :B TO :C)` now resolves the multi-label
  endpoint to `OneOf([idx_A, idx_B])` when each label is a distinct single-
  label node type. The `EdgeEndpointDef::one_of` constructor sorts, dedupes,
  and collapses singletons to `NodeType` on both type models;
  `GraphTypeDef::validate_ref` rejects malformed OneOf payloads (singleton,
  unsorted, duplicated, out-of-range) as defense in depth for rkyv/serde
  decode paths. SHOW EDGE TYPES renders OneOf endpoints as comma-joined
  member labels (round-trippable through `parse()` + re-execute). GTYP V3
  and WAL `EdgeTypeAddedV2` extend in place (both dev-line, born post
  v1.0.0); legacy V1 / V2 GTYP and `EdgeTypeDefV1` WAL payloads remain
  OneOf-blind by design and decode unchanged. Closes Mnemosyne M0-T04
  blocker (U12).
- Non-leading `MATCH` clauses now lower as sequential binding-table extensions
  in `selene-gql`, covering cross-product and correlated continuation shapes.
- Scalar `VALUE { ... }` subqueries in `selene-gql`, including correlated
  read-query bodies, static ISO §20.6 shape checks, and empty-result `NULL`.
- Inline `CALL { ... }` table subqueries in `selene-gql`, including implicit
  variable-scope correlation, `YIELD`/`YIELD ... AS`, and per-row Cartesian
  result composition.
- Bounded variable-length edge patterns in `selene-gql` for `WALK` matches,
  including `JoinTree::Repeat`, `LIST<EdgeRef>` group-variable binding, zero-hop
  results, per-hop edge predicates, and cancellation checks.
- `MATCH` path selectors in `selene-gql` for `ALL`, `ANY`, `ALL SHORTEST`,
  and `ANY SHORTEST` over fixed and bounded variable-length path patterns.
- Restrictive `MATCH` path modes in `selene-gql` for explicit `WALK`, `TRAIL`,
  `SIMPLE`, and `ACYCLIC`, including ordered path contributors for fixed and
  bounded variable-length path validation.
- Wave 3 variable-length relationships are complete: unbounded edge
  quantifiers (`*`, `+`, `{m,}`), questioned edges (`?`), and the ISO §16.4
  legality matrix now execute in `selene-gql` with hard `max_quantifier`
  backstops.
- Shared `selene-gql` `CallPlanCache` for repeated top-level procedure
  `CALL` statements across short-lived sessions, with graph-id and
  schema-version keyed invalidation plus a cache-hit metric.
- Feature-gated `metrics` facade with query, commit, persistence, recovery,
  cancellation, vector search, algorithm, and graph-size metrics.
- `EXCEPT`, `EXCEPT ALL`, `INTERSECT`, `INTERSECT ALL`, and `OTHERWISE`
  set-operation runtime support per ISO GQL §14, including `RuntimeEqKey`
  grouping semantics and a configurable implementation-defined set-op key cap.
- Named vector indexes for `selene-vector-pack`, including
  `vector.create_index`, `vector.drop_index`, and `vector.list_indexes`.
  The default HNSW and IVF indexes remain compatibility anchors for v1.0
  WAL payloads and snapshot sections.
- `StatementOutput::Written` write metadata for committed GQL catalog and
  data mutations, including optional rows for write statements with `RETURN`.
- `Session::flush()` and `DurableProvider::flush()` for explicit durability
  barriers over commit-critical providers.
- Live `CoreProvider` WAL writes through `SharedGraphBuilder::with_wal(...)`
  and `SharedGraph::from_graph_with_wal(...)`.
- `parse_many(...)` for semicolon-separated multi-statement GQL scripts.
- `Session::start_transaction`, `Session::commit_transaction`, and
  `Session::rollback_transaction` for explicit transaction control without
  parser round-trips. Returns `TransactionOutcome` / `RollbackOutcome` with
  changes, durable_at, statement_count, and duration metadata.
- `write_e2e` benches for explicit-transaction Rust API commit and rollback
  paths.
- `selene-graph` runtime schema-version epoch for committed schema changes,
  exposed through `SharedGraph::schema_version()`.
- `selene-gql` opt-in Session plan caching through
  `Session::with_plan_cache(...)` and `Session::execute_source(...)`. The
  cache invalidates on schema-version bumps and skips CALL plans until
  procedure registries expose an epoch.
- `write_e2e/gql_insert_single_node_cached` and
  `write_e2e/gql_insert_single_node_cached_with_schema_churn` benches for the
  Session plan-cache hot path.
- **Parameter binding** through `Session::bind_parameter(name, value)`,
  `Session::clear_parameter(name)`, and `Session::clear_parameters()`.
  `$name` references are resolved from the session parameter map end-to-end;
  unreferenced parameters are ignored, runtime type mismatches are strict
  `ExecutorError::InvalidParameterType` errors with GQLSTATUS 22G03, and typed
  declarations such as `$id :: INTEGER` remain a v1.2 follow-up.
- **Evaluator completeness (16/18)**: `runtime/evaluator/` now executes `LIKE`,
  `BETWEEN`, `IS` checks (7 sub-kinds), `CASE`, 0-indexed list access, record
  literals, `PROPERTY_EXISTS`, `ALL_DIFFERENT`, `SAME`, a closed
  case-insensitive scalar function list of 15 functions, and binary `POWER`,
  `XOR`, concat, `CONTAINS`, `STARTS WITH`, and `ENDS WITH`. `IS NORMALIZED`
  returns `ExecutorError::FeatureNotInV1_1` with GQLSTATUS 42N01 pending v1.2
  Unicode normalization work; `Exists` and `CountSubquery` are deferred to
  BRIEF-116b.
- Evaluator now executes `EXISTS { MATCH ... }` and `COUNT { MATCH ... }`
  subqueries (ISO §19.4 EXISTS predicate; COUNT subquery is a selene-db
  dialect extension). 18/18 v1.0 evaluator stubs now closed (16 by BRIEF-116;
  2 by BRIEF-116b). Outer-binding references in subquery patterns are resolved
  at lowering and seeded at evaluation.
- `ExecutorError::{UnknownFunction, FunctionArityMismatch,
  InvalidFunctionModifier, FeatureNotInV1_1}` for scalar dispatch and scoped
  v1.1 evaluator feature errors.
- `runtime/evaluator/` submodule layout split into `mod.rs`, `binary_ops.rs`,
  `predicates.rs`, `scalar_fns.rs`, `case.rs`, and `collections.rs`.
- Cooperative cancellation and per-statement resource limits:
  `CancellationToken`, `Session::with_cancellation_token(...)`,
  `Session::with_deadline(Instant)`, and `Session::with_row_cap(usize)`.
  Cancellation is cooperative across executor pipeline checkpoints,
  procedure-pack adapters, algorithm hot loops, and bulk vector payload
  construction. Row caps apply only to outermost statement output rows.
  New `ExecutorError::{Cancelled, Timeout, RowCapExceeded}` map to
  implementation-defined GQLSTATUS codes `5GQL2`, `5GQL3`, and reused
  `5GQL1`, respectively.
- GQL surface completeness: `SHOW INDEXES` now lists built-in property indexes
  only (vector indexes remain available through `CALL vector.list_indexes()`),
  `SHOW PROCEDURES` lists registered procedures, `EXPLAIN <statement>` returns
  an indented plan dump without executing the inner statement, and
  `selene.feature_status` is registered in selene-pack. The analyzer now
  bounds value-expression recursion at depth 256 with
  `AnalysisError::RecursionLimitExceeded` (GQLSTATUS `5GQL1`). Named
  `CREATE INDEX` / `DROP INDEX` DDL lowering remains deferred pending
  storage-layer named-index support.
- Catalog type declarations now persist and enforce `DEFAULT`, `IMMUTABLE`,
  and `STRICT` / `WARN` validation modes. CORE/GTYP snapshots and schema WAL
  changes use additive v2 encodings while preserving legacy recovery; relaxed
  writes emit implementation-defined warning GQLSTATUS `01N01`.
- `vector.search` and `vector.ivf_search` now accept an optional nullable
  `metric` argument (`cosine`, `l2`, or `dot`) that overrides query-time
  scoring. HNSW overrides score against the existing build-time topology; IVF
  supports every override except Cosine on non-Cosine-built indexes, where
  reconstructed-norm side data is absent and the call returns GQLSTATUS
  `22G03`. IVF top-k search now uses a bounded heap and reusable per-thread
  scratch buffers.
- `IStrAdmissionPolicy` and `Session::with_istr_admission_policy(...)` let
  embedders opt into graceful interner-cap fallback at runtime string-admission
  boundaries. The default remains `Reject`; `FallbackToExternal` carries
  eligible over-cap text as `Value::ExternalString`.
- Procedure metadata now includes procedure-scope descriptions, signature
  `since_version`, parameter descriptions and `default_doc`, and output-column
  descriptions. `SHOW PROCEDURES` now returns seven columns:
  `name`, `tier`, `mutability`, `signature`, `description`, `since_version`,
  and `capability_required`.
- Conformance-integrity coverage for v1.1: Pattern beta ISO feature IDs
  `GQ08`, `GQ12`, `GQ13`, `GQ20`, `GE04`, `GE05`, `GE07`, `GF13`, and
  `GH02` are now registered and covered by corpus cases, and
  `STDDEV_POP` / `STDDEV_SAMP` execute with an O(1) Welford accumulator.
- `ExecutorWarning`, `WarningSink`, and
  `Session::with_warning_sink(...)` for opt-in runtime warning collection.
  Aggregate NULL elimination now emits ISO GQLSTATUS `01G11` once per
  aggregate expression per statement; sessions without a sink silently discard
  warnings.

### Fixed

- Align `POWER` overflow GQLSTATUS handling with ISO/IEC 39075:2024 §20.22 GR11:
  overflow now maps to `22003` numeric-value-out-of-range, while `2201F`
  invalid-argument-for-power-function is reserved for the zero-base negative
  exponent and negative-base non-integral exponent cases.

### Changed

- ISO GQL error-code conformance: remapped `GqlStatus` constants and
  `miette` diagnostic tags to GQLSTATUS values per ISO/IEC 39075:2024
  section 23.1 Table 8. Public-facing `as_str()` output changes for 9
  status constants (for example, `42601 -> 42001`, `42883 -> 22G03`,
  `0A000 -> 42N01`). Added `GqlStatus::IN_FAILED_TRANSACTION` (`25N02`)
  and `GqlStatus::class()`. No source changes are required for downstream
  consumers using `GqlStatus::SYNTAX_ERROR` and related constants by name;
  consumers comparing raw 5-character codes must update strings.
- Program-limit errors from `IStrCapExceeded`, `PayloadTooLarge`,
  `TooManySections`, and `SectionTooLarge` now report GQLSTATUS `5GQL1`
  instead of the SQLSTATE-shaped `54000`.
- Removed phantom Pattern alpha support claims for path selectors
  (`G015`-`G018`) and graph management (`GC04`/`GC05`); these surfaces now
  reject with GQLSTATUS `42N01` before execution instead of reaching planner
  or runtime implementation-defined failures.
- Collapsed residual implementation-defined `XX500`/`XX501`/`XX502` status
  emissions to single GQLSTATUS `5GQL0`, with diagnostic-detail tags for graph
  mutation, durability flush, and generic implementation-defined failures.
  Residual `22023` emissions in core, graph, and persist now report `22G03`.
- GQL record equality now propagates NULL and NaN through nested record fields
  in the runtime `=` comparator while preserving `Value::PartialEq` and
  runtime row-key structural equality for deduplication.
- Runtime data exceptions now carry a `DataExceptionSubclass` selected at the
  emission site. Arithmetic overflow reports `22003`, division by zero
  reports `22012`, invalid power arguments report `2201F`, invalid value-type
  paths report `22G03`, incomparable ordering reports `22G04`, and record /
  graph-property subclasses use `22G0X`, `22G0M`, `22G0S`, and `22G0T` where
  live paths exist. GQLSTATUS is read from `gqlstatus()`; the stable miette
  diagnostic tag remains broad for the parent data-exception variant.
- Transaction and graph-type surfaces now emit live ISO Table 8 classes:
  nested `START TRANSACTION` returns `25G01`, write attempts in read-only
  transaction contexts return `25G03`, invalid transaction termination returns
  `2D000`, bare node delete with incident edges returns `G1001`, and closed
  graph schema/type violations return `G2000`.

## [1.0.0] — 2026-05-16

First stable release. selene-db is now usable as a Rust dependency for
embedding a property graph engine that targets ISO/IEC 39075:2024 (GQL)
conformance. The public API surface across `selene-core`,
`selene-graph`, `selene-persist`, `selene-gql`, and `selene-pack` is
considered stable: subsequent 1.x releases will maintain
backwards-compatible additions and reserve breaking changes for major
version bumps.

### Highlights

- **Strict ISO/IEC 39075:2024 GQL** parser, semantic analyzer, planner,
  optimizer, and executor. The GQL Flagger (Clause 24.6) rejects
  non-standard or unclaimed constructs at parse time. No Cypher, no
  SQL, no SPARQL grammar leaks into the engine.
- **In-memory property graph** with copy-on-write isolation built on
  `ArcSwap` + `parking_lot::RwLock` + `imbl` persistent collections +
  `RoaringBitmap` label indexes + typed secondary indexes. Single graph
  write-lock with lock-free reads; strict-serializable transaction
  isolation.
- **Write-ahead log** (`SLDB` magic) and **rkyv-archived snapshots**
  (`SLSN` magic) with two-step recovery; the persistence crate is
  graph-blind (operates on `&[Change]`) so graph types can evolve
  independently.
- **Procedure-pack registry** with JSON Schema 2020-12 manifest
  validation, typestate-sealed activation, and a single mutation funnel
  shared between graph writes and pack lifecycle audit (atomic via the
  WAL).
- **Graph algorithm library** with 15 algorithms across four families
  (structural, pathfinding, centrality, community), exposed through 19
  `algo.*` procedures via `selene-algorithms-pack`. Rayon-parallel
  implementations for APSP, betweenness, PageRank, and triangle count.
- **Vector index extension** with HNSW and IVF providers, SQ8/PQ/OPQ
  quantization, and 9 `vector.*` procedures via `selene-vector-pack`.
- **Snapshot-protected** runtime surfaces: planner, executor,
  procedure-pack, and algorithm outputs are pinned by golden snapshots
  for drift detection (D21 snapshot harness pattern).
- **Engineering posture**: `#![forbid(unsafe_code)]` workspace-wide,
  `missing_docs = "deny"`, 700 LOC per-file cap, rustls-only TLS in
  transitive dependencies, no hand-rolled crypto / TLS / async / serde
  primitives.

### Crates

- `selene-core` — foundation types: `Value`, `IStr` interner,
  `PropertyMap`, `LabelSet`, schema types, `Codec`, `Origin`,
  `Changeset`, `feature_register` (ISO feature claims).
- `selene-graph` — in-memory property graph: storage primitives,
  `Mutator` write funnel, label/typed/composite indexes,
  `IndexProvider` extension trait, `GraphTypeDef` runtime binding.
- `selene-persist` — WAL format (`SLDB`), snapshot format with
  TLV-tagged sections (`SLSN`), recovery pipeline. Graph-blind.
- `selene-gql` — pest GQL grammar, AST, semantic analyzer, planner,
  rule-based optimizer (13 rules), row-at-a-time executor,
  `ProcedureRegistry` trait, GQL Flagger.
- `selene-pack` — procedure-pack registry, manifest validator,
  typestate activation state machine, atomic mutation-funnel audit,
  blake3 content hashing, platform built-ins (`selene.health`,
  `selene.create_index`, `selene.drop_index`, `selene.pack.history`,
  `selene.feature_status`).
- `selene-algorithms` — `GraphProjection` + `ProjectionCatalog`
  foundation, four algorithm families. Independent of `selene-gql`.
- `selene-algorithms-pack` — procedure-pack adapters that expose
  `selene-algorithms` through GQL `CALL`.
- `selene-vector` — opt-in HNSW and IVF vector indexes with search,
  mutation replay, snapshots, quantization, `IndexProvider`
  registration.
- `selene-vector-pack` — procedure-pack adapters for vector search,
  mutation, bulk mutation, IVF search, and IVF stats through `CALL`.
- `selene-testing` — shared fixtures, synthetic graph generators, and
  pure-mirror snapshot-harness DSLs. Dev-only.

### ISO/IEC 39075:2024 conformance

selene-db targets **minimum conformance** plus a curated subset of
optional features:

- Mandatory data types: `STRING`, `BOOLEAN`, `INT`, `FLOAT`.
- Optional types behind ISO feature gates: date/time, decimal, list,
  record, path, references.
- Both `GG01` (open graph) and `GG02` (closed graph) are supported;
  per-graph choice.
- Default transaction isolation is **serializable** (Clause 4.6).
- Implementation-defined hooks claimed: `IW010` (external procedures
  via `CALL`), `IV011` (dynamic property value type), `ID001` /
  `IW002` / `ID003` (principals, authorization, and privileges as
  embedder responsibilities), `IE002` / `IE004` (transaction
  isolation).
- No wire format is in scope (Clause 4.2.3 is explicit). Embedders pick
  their own transport.

The `feature_register` in `selene-core` is the authoritative list of
claimed Implication-table features. The Flagger rejects unclaimed
constructs at parse time so embedders cannot accidentally rely on
non-standard syntax.

### Snapshot and WAL format

- WAL magic: `SLDB`. Append-only log of `Change` records with
  configurable `SyncPolicy` (synchronous, batched, or unsynced for
  testing).
- Snapshot magic: `SLSN`. rkyv-archived TLV sections; producer-tagged
  for graph core (`GRPH`), vectors (`VECS`), quantization (`QUNT`,
  `CQNT`, `IPQB`), IVF (`IVF1`), OPQ rotation (`ROTN`), and PQ
  training (`PQTC`). Extensions register their own section tags.
- Two-step recovery: load snapshot, replay WAL from snapshot's last
  seqno. Selene preserves byte-parity for prior snapshot section
  versions on load while writing the current version.

### Performance posture

Benchmarks are local-only via `scripts/run-benches.sh` (criterion and
iai-callgrind, sequential, with `mimalloc` as the global allocator).
[`BENCHMARKS.md`](BENCHMARKS.md) is the committed, dated measurement
record. Selected v1.0.0 headlines (Apple M5, 100k-scale full profile):

| Surface | Measurement |
|---|---|
| `graph_node_fetch` | ~2.10 ns (columnar storage, lock-free read) |
| `graph_typed_index_point` | ~4.53 ns (flat-curve `Cow` tri-state) |
| `gql_analyze_corpus/m5c` | ~5.32 µs (full analyzer pipeline) |
| `algo_betweenness 100k parallel speedup` | ~2.40× over sequential |
| `vector_ivf_search` | ~2.88 µs per query |

See [`docs/performance.md`](docs/performance.md) for the full surface,
tuning knobs, and methodology.

### Documentation

This release introduces a full user-facing documentation set under
[`docs/`](docs/):

- [Getting started](docs/getting-started.md)
- [Embedding selene-db](docs/embedding-guide.md)
- [GQL reference](docs/gql-reference.md)
- [Architecture](docs/architecture.md)
- [Extension guide](docs/extension-guide.md)
- [Vector search](docs/vector-search.md)
- [Graph algorithms](docs/graph-algorithms.md)
- [Persistence and recovery](docs/persistence-and-recovery.md)
- [Performance](docs/performance.md)
- [Contributing](docs/contributing.md)

The README is now focused on evaluation and orientation; depth lives in
the documentation pages above.

### Stability guarantees

The following surfaces are stable starting with 1.0.0:

- Public types and traits in `selene-core` (`Value`, `IStr`,
  `PropertyMap`, `LabelSet`, `Change`, `Codec`).
- Public types and methods in `selene-graph` (`SharedGraph`,
  `Mutator`, `IndexProvider`, write-transaction commit flow).
- Public types and methods in `selene-persist` (WAL format, snapshot
  format, recovery API).
- Public types and methods in `selene-gql` (`parse`, `analyze`,
  `plan`, `execute_statement`, `Session`, `ProcedureRegistry`).
- Public types and methods in `selene-pack` (manifest schema,
  `ExternalProcedurePack`, lifecycle events).
- Wire formats for WAL and snapshot sections (read-side compatibility
  preserved across 1.x).
- The 13 optimizer rules and their static effects on plans.
- The 19 `algo.*` and 9 `vector.*` procedure surfaces.

### Platform support

| Platform | Status |
|---|---|
| Linux (x86_64, aarch64) | Primary deployment target |
| macOS (Apple Silicon, Intel) | Primary development target |
| Windows | Out of scope |

### Known deferrals (post-1.0.0)

The following items are intentionally deferred and tracked for future
1.x releases:

- Louvain parallelization (currently sequential).
- Edge-index planner support (typed/composite indexes for edges
  currently return `Linear` selectivity).
- Analyzer recursion-depth bound (parser is bounded at 64; analyzer
  binding is not yet contractually bounded).
- Mutation/`MATCH` planner threading of `BindingId` (currently uses
  per-statement context).
- OPQ rotation inner-allocation tightening.
- Fresh extension crates beyond `selene-vector` and `selene-algorithms`.

[1.0.0]: https://github.com/jscott3201/selene-db/releases/tag/v1.0.0
