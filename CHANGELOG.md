# Changelog

All notable changes to selene-db are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- `ExecutorError::{UnknownFunction, FunctionArityMismatch,
  InvalidFunctionModifier, FeatureNotInV1_1}` for scalar dispatch and scoped
  v1.1 evaluator feature errors.
- `runtime/evaluator/` submodule layout split into `mod.rs`, `binary_ops.rs`,
  `predicates.rs`, `scalar_fns.rs`, `case.rs`, and `collections.rs`.

### Changed

- ISO GQL error-code conformance: remapped `GqlStatus` constants and
  `miette` diagnostic tags to GQLSTATUS values per ISO/IEC 39075:2024
  section 23.1 Table 8. Public-facing `as_str()` output changes for 9
  status constants (for example, `42601 -> 42001`, `42883 -> 22G03`,
  `0A000 -> 42N01`). Added `GqlStatus::IN_FAILED_TRANSACTION` (`25N02`)
  and `GqlStatus::class()`. No source changes are required for downstream
  consumers using `GqlStatus::SYNTAX_ERROR` and related constants by name;
  consumers comparing raw 5-character codes must update strings.

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
