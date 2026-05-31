# selene-db

An embeddable property graph engine for Rust, built to the ISO/IEC 39075:2024 GQL standard.

`selene-db` is a multi-crate Rust workspace that ships a **single native graph engine** — one cohesive core with graph algorithms inlined as a first-class mandatory crate. There is no extension or procedure-pack system. The query language is **strict ISO GQL**: no Cypher, no SQL, no SPARQL grammar in the engine. Non-graph capabilities (time-series, vectors, RDF, GraphRAG) are externalized to separate dedicated projects, never in-tree extensions.

The engine is library-only: no transport, no auth, no server. Embedders take the workspace crates as dependencies and run the engine in-process.

## At a glance

- **ISO/IEC 39075:2024 GQL** parser, semantic analyzer, planner, optimizer, and row-at-a-time executor.
- **In-memory property graph** with copy-on-write isolation: `ArcSwap` + `parking_lot::RwLock` + `imbl` persistent collections + `RoaringBitmap` label indexes + typed secondary indexes.
- **Strict-serializable** transaction isolation; single graph write-lock with lock-free reads.
- **Write-ahead log** (`SLDB` magic) and **rkyv-archived snapshots** (`SLSN` magic) with two-step recovery; the persistence crate never sees the graph types directly.
- **Native procedure registry**: the single frozen `BuiltinProcedureRegistry` binds ISO `CALL` (IW010) over 24 native procedures — 5 platform built-ins (`selene.{health,feature_status,verify,create_index,drop_index}`) plus the 19 `algo.*` procedures — with no loadable-pack machinery; index DDL routes through the one mutation funnel.
- **Graph algorithm library**: a mandatory first-class crate spanning structural (WCC / SCC / topological sort / articulation points / bridges), pathfinding (Dijkstra / SSSP / APSP), centrality (PageRank / Brandes betweenness), and community (label propagation / Louvain / triangle count). Exposed both as a native Rust API and as the 19 `algo.*` procedures.
- **Forbids unsafe Rust** workspace-wide; `missing_docs = "deny"`; per-file LOC cap; `rustls`-only TLS posture in transitive dependencies.

## Capabilities

| Capability                       | Backing crate                                                                                              | How it is exposed                                                                                          |
| :------------------------------- | :--------------------------------------------------------------------------------------------------------- | :--------------------------------------------------------------------------------------------------------- |
| ISO/IEC 39075:2024 GQL           | [`selene-gql`](crates/selene-gql)                                                                          | Parser, semantic analyzer, planner, optimizer, row-at-a-time executor.                                     |
| In-memory property graph         | [`selene-graph`](crates/selene-graph)                                                                      | Copy-on-write snapshots, label indexes, typed property indexes, composite indexes, and the mutation funnel. |
| Strict-serializable transactions | [`selene-graph`](crates/selene-graph)                                                                      | Single write lock for mutation; lock-free read snapshots through `ArcSwap`.                                |
| Persistence                      | [`selene-persist`](crates/selene-persist)                                                                  | Graph-blind WAL (`SLDB`) and snapshot (`SLSN`) formats with MANIFEST-led recovery.                         |
| Native procedures (`CALL`)       | [`selene-gql`](crates/selene-gql)                                                                          | The frozen `BuiltinProcedureRegistry`: 5 platform built-ins plus 19 `algo.*` procedures over ISO `CALL`.   |
| Graph algorithms                 | [`selene-algorithms`](crates/selene-algorithms)                                                            | `GraphProjection` algorithms, the native Rust API, and the `CALL algo.*` binding via the built-in registry. |
| Test corpus mirrors              | [`selene-testing`](crates/selene-testing)                                                                  | Shared fixtures and pure-mirror snapshot DSLs consumed by crate integration tests.                         |

## Workspace layout

| Crate                                                              | Purpose                                                                                                                            |
| :----------------------------------------------------------------- | :--------------------------------------------------------------------------------------------------------------------------------- |
| [`selene-core`](crates/selene-core)                                | Foundation types: `Value`, `IStr` interner, `PropertyMap`, `LabelSet`, schema types, `Codec`, `Origin`, `Changeset`.               |
| [`selene-graph`](crates/selene-graph)                              | In-memory property graph: storage, `Mutator` write funnel, label/typed/composite indexes, `IndexProvider` hook, `GraphTypeDef`.    |
| [`selene-persist`](crates/selene-persist)                          | WAL format, snapshot format with TLV-tagged sections, recovery pipeline. Graph-blind: takes `&[Change]`, returns `RecoveryResult`. |
| [`selene-gql`](crates/selene-gql)                                  | Pest GQL grammar, AST, semantic analyzer, planner, rule-based optimizer, executor, `ProcedureRegistry` trait, and its sole frozen impl `BuiltinProcedureRegistry`. |
| [`selene-algorithms`](crates/selene-algorithms)                    | Mandatory first-class crate: `GraphProjection` + `ProjectionCatalog` foundation, structural / pathfinding / centrality / community families, and the native Rust API (free functions + the `GraphAlgorithms` extension trait). |
| [`selene-testing`](crates/selene-testing)                          | Shared test fixtures, synthetic graph generators, pure-mirror snapshot-harness DSLs. Consumed via `[dev-dependencies]`.            |

All crates are mandatory; there is no extension or procedure-pack system. Graph algorithms are a first-class crate exposed natively and through `CALL algo.*`, and the dependency direction is strictly linear (`core → graph → algorithms → gql`; `selene-algorithms` never imports `selene-gql`).

## Quickstart

`selene-db` is library-only. An embedder takes the workspace crates as path dependencies and runs the engine in-process:

```toml
# Cargo.toml
[dependencies]
selene-core = { path = "path/to/selene-db/crates/selene-core" }
selene-graph = { path = "path/to/selene-db/crates/selene-graph" }
selene-gql = { path = "path/to/selene-db/crates/selene-gql" }
selene-persist = { path = "path/to/selene-db/crates/selene-persist" }
```

Direct mutation via the graph crate's write API:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, intern};
use selene_graph::SharedGraph;

let graph = SharedGraph::new(GraphId::new(1));
let person = intern("Person").unwrap();
let name = intern("name").unwrap();

let mut tx = graph.begin_write();
let mut props = PropertyMap::new();
props.set(name, Value::String(intern("Ada").unwrap())).unwrap();
tx.mutator()
    .create_node(LabelSet::single(person), props)
    .unwrap();
tx.commit().unwrap();
```

End-to-end GQL execution over the graph built above:

```rust
use selene_gql::{
    EmptyProcedureRegistry, StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_core::{Value, intern};

let registry = EmptyProcedureRegistry;
let statement = parse("MATCH (p:Person) RETURN p.name").unwrap();
let analyzed = analyze(statement, &registry, None).unwrap();
let planned = plan(&analyzed, &registry).unwrap();
let mut session = selene_gql::Session::new(&graph);
let output = execute_statement(&planned, &mut session, &registry).unwrap();

let StatementOutput::Rows(rows) = output else {
    panic!("query should return rows");
};
assert_eq!(rows.row_count(), 1);
assert_eq!(
    rows.rows()[0].values()[0],
    Value::String(intern("Ada").unwrap())
);
```

See [docs/getting-started.md](docs/getting-started.md) for a complete walk-through.

## ISO/IEC 39075:2024 conformance posture

`selene-db` targets **minimum conformance** plus a curated subset of optional features. The full feature register (in `selene-core`) declares which Implication-table optional features the engine claims; the **GQL Flagger** (ISO 39075 Clause 24.6) rejects non-standard or unclaimed constructs at parse time.

- Mandatory data types: `STRING`, `BOOLEAN`, `INT`, `FLOAT`. Optional types (date/time, decimal, list, record, path, references) ship under their ISO feature gates.
- Both **GG01** (open graph) and **GG02** (closed graph) are supported; per-graph choice.
- Default transaction isolation is **serializable** (Clause 4.6); the engine uses strict-serializable under a single write lock with lock-free reads.
- Implementation-defined hooks claimed: `IW010` (external procedures via `CALL`), `IV011` (dynamic property value type), `ID001` / `IW002` / `ID003` (principals / authzn / privileges as embedder responsibilities), `IE002` / `IE004` (transaction isolation).
- No wire format is in scope (Clause 4.2.3 is explicit). Embedders pick their own transport.

## Performance

Recent measurements on Apple M5 (sequential criterion via `scripts/run-benches.sh`):

- `graph_node_fetch`: **2.10 ns** — flat O(1) columnar fetch.
- `graph_typed_index_point`: **4.53 ns** — flat across scales via tri-state `Cow<RoaringBitmap>` lookup.
- `gql_analyze_corpus/m5c`: **5.32 µs** semantic analysis on the representative corpus.
- `betweenness` @ 100k nodes: 264.7 ms sequential, **110.2 ms** parallel (2.40× speedup).

See [BENCHMARKS.md](BENCHMARKS.md) for the full table and [docs/performance.md](docs/performance.md) for tuning knobs.

## Documentation

- [Getting started](docs/getting-started.md) — install, first query, common patterns.
- [Embedding selene-db](docs/embedding-guide.md) — using selene-db as a library in your application.
- [GQL reference](docs/gql-reference.md) — the ISO GQL surface selene-db supports.
- [Architecture](docs/architecture.md) — crate layout, threading model, design decisions.
- [Graph algorithms](docs/graph-algorithms.md) — the native `selene-algorithms` API and the algorithms exposed through `algo.*` procedures.
- [Persistence and recovery](docs/persistence-and-recovery.md) — WAL, snapshots, recovery flow.
- [Performance](docs/performance.md) — benchmarks, tuning knobs.
- [Contributing](docs/contributing.md) — dev setup, CI gates, code style.

## Engineering

`selene-db` is built marathon-style: correctness, performance, and a cohesive single-native-engine architecture over near-term shortcuts. The workspace forbids `unsafe_code`, denies `missing_docs`, caps source files at 700 LOC, pins TLS to `rustls`-only in transitive dependencies, and disallows hand-rolled crypto / TLS / async runtime / serialization primitives. Conventional commits with crate-or-component scopes are required. See [docs/contributing.md](docs/contributing.md) for the full posture, CI gates, and review workflow.

## Platform support

| Platform                       | Status                                                                |
| :----------------------------- | :-------------------------------------------------------------------- |
| Linux (x86_64, aarch64)        | Primary deployment target.                                            |
| macOS (Apple Silicon, Intel)   | Primary development target; CI parity for `fmt`, `clippy`, `test`.    |
| Windows                        | Out of scope.                                                         |

## Licensing and attribution

Dual-licensed under **MIT OR Apache-2.0** at the embedder's choice (`LICENSE-MIT`, `LICENSE-APACHE`).

- `NOTICE` — Apache-2.0-style attribution naming third-party copyright holders for bundled or adapted code.
- `THIRDPARTY.md` — auto-generated from `Cargo.lock` via `cargo-about`; CI-gated against drift.

When a third-party source is adapted at file level, the affected file carries an `// Adapted from <upstream>@<version-or-commit> (<SPDX>)` attribution comment.
