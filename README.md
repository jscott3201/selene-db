# selene-db

An embeddable property graph engine for Rust, built to the ISO/IEC 39075:2024 GQL standard.

`selene-db` is a multi-crate Rust workspace that ships a small, high-performance graph core with a deliberate extension boundary. The query language is **strict ISO GQL**: no Cypher, no SQL, no SPARQL grammar in the engine. Capabilities the standard does not define — vectors, time-series, RDF, GraphRAG, full-text — live in opt-in extension crates that plug in through stable interfaces.

The engine is library-only: no transport, no auth, no server. Embedders take the workspace crates as dependencies and run the engine in-process.

## At a glance

- **ISO/IEC 39075:2024 GQL** parser, semantic analyzer, planner, optimizer, and executor.
- **In-memory property graph** with copy-on-write isolation: `ArcSwap` + `parking_lot::RwLock` + `imbl` persistent collections + `RoaringBitmap` label indexes + typed secondary indexes.
- **Strict-serializable** transaction isolation; single graph write-lock with lock-free reads.
- **Write-ahead log** (`SLDB` magic) and **rkyv-archived snapshots** (`SLSN` magic) with two-step recovery; the persistence crate never sees the graph types directly.
- **Procedure-pack registry**: JSON-manifest-validated, typestate-sealed activation; one mutation funnel for both graph writes and lifecycle audit, atomic via the WAL.
- **Graph algorithm library**: 15 public surfaces across structural (WCC / SCC / topological sort / articulation points / bridges), pathfinding (Dijkstra / SSSP / APSP), centrality (PageRank / Brandes betweenness), and community (label propagation / Louvain / triangle count). Each runs over a frozen `GraphProjection` with cached CSR adjacency; algorithms are pure functions of `&GraphProjection`.
- **Snapshot-protected** runtime surfaces: planner, executor, procedure-pack, and algorithm outputs are pinned by golden snapshots for drift detection.
- **Forbids unsafe Rust** workspace-wide; `missing_docs = "deny"`; per-file LOC cap; `rustls`-only TLS posture in transitive dependencies.

## Workspace layout

| Crate | Purpose |
|---|---|
| [`selene-core`](crates/selene-core) | Foundation types: `Value`, `IStr` interner, `PropertyMap`, `LabelSet`, schema types, `Codec`, `Origin`, `Changeset`. |
| [`selene-graph`](crates/selene-graph) | In-memory property graph: storage primitives, `Mutator` write funnel, label/typed/composite indexes, `IndexProvider` extension hook, `GraphTypeDef` runtime binding. |
| [`selene-persist`](crates/selene-persist) | WAL format, snapshot format with TLV-tagged sections, recovery pipeline. Graph-blind: takes `&[Change]`, returns `RecoveryResult`. |
| [`selene-gql`](crates/selene-gql) | Pest GQL grammar, AST, semantic analyzer, planner, rule-based optimizer, row-at-a-time executor, `ProcedureRegistry` trait. |
| [`selene-pack`](crates/selene-pack) | Procedure-pack registry, manifest validator (JSON Schema 2020-12 gates), typestate activation state machine, atomic mutation-funnel audit, canonical blake3 content hashing, and platform built-ins (`selene.health`, `selene.create_index`, `selene.drop_index`, `selene.pack.history`). |
| [`selene-algorithms`](crates/selene-algorithms) | `GraphProjection` + `ProjectionCatalog` foundation, four algorithm families (structural / pathfinding / centrality / community), D21 snapshot harness. Independent of the GQL crate. |
| [`selene-testing`](crates/selene-testing) | Shared test fixtures, synthetic graph generators, pure-mirror snapshot-harness DSLs for the planner / executor / procedure-pack / algorithm corpora. Consumed via `[dev-dependencies]`. |

Opt-in extension crates depend on the workspace crates plus the procedure-pack and `IndexProvider` hooks. The first is **`selene-vector`** (HNSW + PolarQuant + vector procedures); spatial, time-series, RDF, GraphRAG, and full-text extensions slot in via the same shape.

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

A minimal session: build a graph, parse-analyze-plan-execute a GQL statement, observe results.

```rust
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
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

Running GQL goes through the `selene-gql` parser, semantic analyzer, planner, and executor — see the crate docs for the embedder-facing API.

## ISO/IEC 39075:2024 conformance posture

`selene-db` targets **minimum conformance** plus a curated subset of optional features. The full feature register (in `selene-core`) declares which Implication-table optional features the engine claims; the **GQL Flagger** (ISO 39075 clause 24.6) rejects non-standard or unclaimed constructs at parse time.

- Mandatory data types: `STRING`, `BOOLEAN`, `INT`, `FLOAT`. Optional types (date/time, decimal, list, record, path, references) ship under their ISO feature gates.
- Both **GG01** (open graph) and **GG02** (closed graph) are supported; per-graph choice.
- Default transaction isolation is **serializable** (clause 4.6); the engine uses strict-serializable under a single write lock with lock-free reads.
- Implementation-defined hooks claimed: `IW010` (external procedures via `CALL`), `IV011` (dynamic property value type), `ID001` / `IW002` / `ID003` (principals / authzn / privileges as embedder responsibilities), `IE002` / `IE004` (transaction isolation).
- No wire format is in scope (clause 4.2.3 is explicit). Embedders pick their own transport.

## Architecture decisions

Twenty-one numbered decisions (`D1`–`D21`) define the workspace shape. They live in `CLAUDE.md` (decision log section) and in per-spec amendment ranges:

- D1 — v1.0 is an embeddable library (no server, transport, or auth).
- D5 — Vectors live in `selene-vector`, not in `selene-graph`. Generally: every non-graph capability lives in an extension crate.
- D7 — Concurrency primitives: `ArcSwap` + `parking_lot::RwLock` + `imbl` copy-on-write.
- D8 — Multi-crate workspace; no umbrella crate; linear dependency direction `core → graph → persist → gql → pack → algorithms`.
- D14 — Snapshots use rkyv archives over sorted-vec intermediates.
- D17 — Procedure tiers: per-tier concrete `Context` structs + per-tier dyn-compatible `Procedure` traits.
- D18 — Procedure-pack lifecycle audit routes through the same mutation funnel as graph writes; no parallel ledger.
- D21 — Snapshot harness pattern: pure-mirror DSL in `selene-testing`, renderer + integration test + golden `.snap` files in the target crate.

## Engineering posture

- **`#![forbid(unsafe_code)]`** workspace-wide.
- **`missing_docs = "deny"`** workspace-wide.
- **700 LOC per-file cap**, enforced by CI.
- **rustls-only TLS** in transitive dependencies, enforced by `cargo-deny`.
- **No hand-rolled crypto, TLS, async runtime, or serialization primitives.**
- **Conventional commits** with crate-or-component scope.
- **Marathon mindset**: correctness, performance, and a stable extension contract over near-term shortcuts.

## CI gates

Every PR exercises the full gate set:

| Gate | What it checks |
|---|---|
| `fmt` | `cargo fmt --all --check`. |
| `clippy (ubuntu, macos)` | `cargo clippy --workspace --all-targets --locked -- -D warnings` on both Linux and macOS. |
| `test (ubuntu, macos)` | `cargo test --workspace --locked --all-features` on both Linux and macOS. |
| `parse-fuzz` | `cargo +nightly fuzz run parse_gql -- -max_total_time=60`. Linux-only (cargo-fuzz requires `x86_64-unknown-linux-gnu`). |
| `cargo-deny` | License allow-list, ban list, source allow-list. |
| `cargo-audit` | Vulnerability advisories against the locked dependency graph. |
| `file-size cap (700 LOC)` | Per-file line-count gate. |
| `no-secret scan` | Baseline secret-pattern grep against tracked source. |
| `bench invocation lint` | Static checks that benches use the sanctioned runner script. |
| `third-party attribution current` | `THIRDPARTY.md` is in sync with `Cargo.lock` (regenerated via `cargo-about`). |

Benchmarks are local-only and run via `scripts/run-benches.sh` — never `cargo bench --workspace`, which can dispatch bench binaries concurrently. iai-callgrind requires Linux + valgrind; the runner degrades to criterion-only on macOS without error.

## Platform support

| Platform | Status |
|---|---|
| Linux (x86_64, aarch64) | Primary deployment target. |
| macOS (Apple Silicon, Intel) | Primary development target; CI parity for `fmt`, `clippy`, `test`. |
| Windows | Out of scope. |

## Licensing and attribution

Dual-licensed under **MIT OR Apache-2.0** at the embedder's choice (`LICENSE-MIT`, `LICENSE-APACHE`).

- `NOTICE` — Apache-2.0-style attribution naming third-party copyright holders for bundled or adapted code.
- `THIRDPARTY.md` — auto-generated from `Cargo.lock` via `cargo-about`; CI-gated against drift.

When a third-party source is adapted at file level, the affected file carries an `// Adapted from <upstream>@<version-or-commit> (<SPDX>)` attribution comment.
