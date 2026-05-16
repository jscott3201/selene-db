# selene-db architecture

This document describes how selene-db is built. It assumes you have read
[`README.md`](../README.md) and want to understand the layering, concurrency
model, persistence design, extension surface, and the numbered architecture
decisions that shape the workspace.

selene-db is an embeddable property graph engine for Rust that targets
ISO/IEC 39075:2024 GQL minimum conformance plus a curated subset of optional
features. The engine is library-only: no transport, no auth, no server.
Embedders pull the workspace crates as path dependencies and run the engine
in-process.

For operational detail on durability and recovery see
[`persistence-and-recovery.md`](persistence-and-recovery.md). For the GQL
surface see [`gql-reference.md`](gql-reference.md). For algorithm and vector
extension surfaces see [`graph-algorithms.md`](graph-algorithms.md) and
[`vector-search.md`](vector-search.md).

---

## 1. Crate dependency graph

The workspace is a flat tree of ten crates with no umbrella facade
(decision D8). `selene-core` is the leaf. Every other crate transitively
depends on it. `selene-testing` is dev-only and is consumed via
`[dev-dependencies]`.

```text
                                 selene-core
                                      |
            +-------------+-----------+-----------+----------------+
            |             |                       |                |
       selene-persist  selene-graph        selene-algorithms  selene-vector
            ^             ^                       ^                ^
            |             |                       |                |
            +------+------+                       |                |
                   |                              |                |
              selene-gql                          |                |
                   ^                              |                |
                   |                              |                |
              selene-pack <---- selene-algorithms-pack             |
                   ^                                               |
                   |                                               |
                   +------------- selene-vector-pack --------------+

   (dev-only)  selene-testing  depends on selene-core, selene-gql, selene-graph
```

| Crate | Role |
|---|---|
| `selene-core` | Foundation types: `Value`, `IStr` interner, `PropertyMap`, `LabelSet`, schema model, `Codec`, `Origin`, `Change`, GQLSTATUS table, ISO feature register. Zero deps on other selene crates. |
| `selene-persist` | Graph-blind WAL (`SLDB` magic) + rkyv-archived snapshots (`SLSN` magic) + two-step recovery. Depends on `selene-core` only. |
| `selene-graph` | In-memory property graph: storage primitives, `Mutator` write funnel, label / typed / composite indexes, `IndexProvider` extension hook, `GraphTypeDef` runtime binding, `SharedGraph` + `WriteTxn`. |
| `selene-gql` | Pest GQL grammar, AST, semantic analyzer, planner, rule-based optimizer, row-at-a-time executor, `ProcedureRegistry` trait, Flagger. Depends on `selene-core` + `selene-graph`. |
| `selene-pack` | Procedure-pack registry, manifest validator (JSON Schema 2020-12 gates), typestate activation state machine, atomic mutation-funnel audit, blake3 content hashing, platform built-ins (`selene.health`, `selene.create_index`, `selene.drop_index`, `selene.pack.history`). Widest dep set: `selene-core` + `selene-persist` + `selene-graph` + `selene-gql`. |
| `selene-algorithms` | `GraphProjection` + `ProjectionCatalog`, four algorithm families (structural / pathfinding / centrality / community), D21 snapshot harness. Independent of `selene-gql`. |
| `selene-algorithms-pack` | Procedure-pack adapter that exposes `selene-algorithms` through GQL `CALL` by registering an external pack with `selene-pack`. |
| `selene-vector` | Opt-in HNSW and IVF vector index extension with search, mutation replay, snapshots, quantization, and `IndexProvider` registration. |
| `selene-vector-pack` | Procedure-pack adapter that exposes vector search, mutation, bulk mutation, IVF search, and IVF stats through GQL `CALL`. |
| `selene-testing` | Shared fixtures, synthetic graph generators, pure-mirror snapshot-harness DSLs for the planner / executor / procedure-pack / algorithm corpora. Consumed via `[dev-dependencies]`. |

The dependency graph is intentionally acyclic with a single sink
(`selene-core`) and a single broad consumer of the runtime layer
(`selene-pack`). Pack-adapter crates (`selene-algorithms-pack`,
`selene-vector-pack`) sit on top so that the runtime crates never grow
non-graph capability surface.

---

## 2. The layered model

A GQL statement flows through six well-typed boundaries before it touches
graph state. Each stage is a pure function over its input plus, where
necessary, a borrowed registry or context.

```text
   source: &str
        |
   parse(source) -> Statement                       [selene-gql::parser]
        |
   analyze(stmt, registry, type_table)              [selene-gql::analyze]
      -> AnalyzedStatement
        |
   plan(&analyzed, registry) -> ExecutionPlan       [selene-gql::plan]
        |
   optimize(plan, &OptimizeContext) -> ExecutionPlan [selene-gql::plan::optimize]
        |
   execute_statement(&plan, &mut Session,           [selene-gql::runtime]
                     &dyn ProcedureRegistry)
      -> StatementOutput
        |
   Mutator (inside WriteTxn) emits Change events    [selene-graph::mutator]
        |
   commit: snapshot publish + WAL append            [selene-graph + selene-persist]
        |
   readers see new ArcSwap snapshot                 [selene-graph::shared]
```

### Parse

`parse(source: &str) -> Result<Statement, ParserError>` and
`parse_with_source` (which preserves source spans for richer diagnostics)
are the only entry points from raw GQL. The parser is a pest 2.8 PEG defined
in `crates/selene-gql/src/parser/grammar.pest`. Rule names mirror the
ISO 39075 grammar names, aligned with opengql/grammar v1.9.0 where the donor
diverged. Strings are single-quoted (Spec 02); double-quoted lexemes are
delimited identifiers. Backtick-delimited identifiers are a selene
extension. Unicode identifiers are first-class.

### Analyze

`analyze(stmt, registry, optional_param_types) -> Result<AnalyzedStatement,
AnalysisError>` runs semantic checks: scope resolution, binding decls
(`BindingDecl`, `BindingUse`), `ExprId` allocation, expression typing
(`ExprTypeTable`), statement-category classification (`ReadOnly`,
`DataModifying`, `CatalogModifying`, `TransactionControl`), and the mutation
write set (`MutationWriteSet`, `WriteSetEntry`). The Flagger runs in this
phase: any construct outside the v1.0 claimed feature register is rejected
with a `GqlStatus` flag.

### Plan

`plan(&analyzed, registry) -> Result<ExecutionPlan, PlannerError>` lowers
the analyzed AST into a pipeline of `PipelineOp` values. Patterns become
`PatternPlan`s, scans become `NodeOrEdgeScan` or `TypedIndexLookup`,
mutations become `MutationOp`s, and procedure calls become `PlannedCall`s.
The planner consults the registry for procedure signatures and the index
catalog for available indexes.

### Optimize

`optimize(plan, &OptimizeContext) -> ExecutionPlan` applies a rule-based
optimizer. Rules in `selene-gql::plan::optimize::Rule` cover filter
placement, projection pruning, join reshaping, and adaptive
`WanderJoinSampler` decisions where statistics are available. The optimizer
is deterministic and total: every input plan has a defined output plan.

### Execute

`execute_statement(&plan, &mut Session, &dyn ProcedureRegistry) ->
Result<StatementOutput, ExecutorError>` is the top-level entry point. It
dispatches on `StatementCategory`:

- `ReadOnly` statements take an `ArcSwap`-published snapshot and execute
  inside a `TxContext::read_only` against that snapshot.
- `DataModifying` and `CatalogModifying` statements either join an active
  explicit transaction or run auto-commit through `SharedGraph::begin_write`
  to acquire the single write lock.
- `TransactionControl` statements (`START TRANSACTION`, `COMMIT`,
  `ROLLBACK`) route into the `pipeline::tx` module.

A statement that produces rows returns `StatementOutput::Rows(BindingTable)`;
a write that ends in `FINISH` (or any DDL/TX op) returns
`StatementOutput::Empty`.

### Mutator and commit

All graph writes funnel through `selene_graph::Mutator`. The mutator builds
a `Vec<Change>` (label diffs, property diffs, node/edge create/delete,
schema mutations). On `WriteTxn::commit` the mutator atomically swaps a new
graph snapshot into the `ArcSwap` and hands the `&[Change]` slice to
`selene-persist` for WAL append. Lifecycle audit events for procedure-pack
activation route through the same funnel; there is no parallel ledger (D18).

### Persistence

`selene-persist` is graph-blind: it takes `&[Change]` and a principal byte
slice, writes a `SLDB`-prefixed WAL entry, and on recovery returns a
`RecoveryResult` (latest snapshot file + a slice of post-snapshot WAL
entries). `selene-graph` plus any registered `IndexProvider`s apply that
result to rebuild in-memory state. See [`persistence-and-recovery.md`] for
operational detail.

---

## 3. Concurrency model

selene-db uses a one-writer-many-readers shape with no allocation on the
read path. The primitives are codified by decision D7.

| Primitive | Where it is used | Why |
|---|---|---|
| `arc_swap::ArcSwap` | `SharedGraph` publishes each committed snapshot as `Arc<SeleneGraph>`. | Readers grab a pointer in a single relaxed load. No lock acquisition, no allocation, no poisoning. |
| `parking_lot::RwLock` | Per-graph write lock; also wraps mutable index state inside providers. | parking_lot is non-reentrant, never poisons, and is materially faster than `std::sync::RwLock`. Lock poisoning is a non-feature for an in-process embedded engine: a panic during commit must abort the process, not leave readers staring at a `PoisonError`. |
| `imbl` persistent collections | Copy-on-write maps and vectors inside `SeleneGraph` (label sets, property maps, adjacency). | Structural sharing means a new snapshot retains O(log n) overlap with its predecessor; a writer can publish without copying the entire graph. |
| `roaring::RoaringBitmap` | Label indexes, deleted-id sets, candidate selections. | Compact, cache-friendly representation for sparse `NodeId` sets. |
| `triomphe::Arc` | Property-value reference counting in selected hot paths. | `triomphe::Arc` is non-`Weak`, single-word, and avoids the `std::sync::Arc` weak-count overhead where weak refs are not used. |

### How a write commits

1. The embedder calls `SharedGraph::begin_write`, which acquires the write
   `RwLock`. Only one writer can hold this lock per graph.
2. The returned `WriteTxn<'g>` exposes a `Mutator` whose methods append
   `Change`s into a transaction-local buffer.
3. On `WriteTxn::commit` (or `commit_with_principal`), the mutator clones
   the current snapshot's persistent data structures, applies the
   `Change` list, and constructs a new `Arc<SeleneGraph>`.
4. The new snapshot is published via `ArcSwap::store`. Readers in flight
   continue against the old snapshot until they drop their `Arc`. New
   readers see the new snapshot on their next `read()`.
5. The mutator hands `&[Change]` plus principal bytes to the WAL writer
   (`selene-persist::WalWriter`) which appends a framed entry under the
   configured `SyncPolicy`.
6. Registered `IndexProvider`s are notified via `on_change(&Change)`. The
   re-entry contract (see `IndexProvider` rustdoc) forbids same-graph
   `begin_write` calls from inside `on_change`.
7. The write lock drops at the end of the commit boundary.

### How a reader holds a snapshot

A read path inside the executor calls `SharedGraph::read()` once, receives
an `Arc<SeleneGraph>`, and holds that `Arc` for the duration of the
statement. No further coordination is needed: the snapshot is immutable, and
the writer cannot mutate the structures the reader sees.

### Why locks don't poison

selene-db never propagates a `LockResult` to user code. parking_lot locks
do not poison on panic; on a panic during commit, the process aborts (the
panic itself surfaces). Readers cannot observe an in-progress write because
they never look at the writer's transaction buffer; they only observe the
last fully-published `ArcSwap` cell.

---

## 4. Persistence model

selene-db separates persistence into two independent file formats, each
with a stable 4-byte magic prefix.

| Format | Magic | Producer | Consumer | Purpose |
|---|---|---|---|---|
| WAL entry | `SLDB` | `selene_persist::WalWriter` | `WalReader`, `recover` | Per-commit framed entry: header (length, principal, flags, checksum) followed by postcard-serialized `Vec<Change>`. |
| Snapshot | `SLSN` | `SnapshotBuilder` | `SnapshotReader` | rkyv-archived snapshot with TLV-tagged sections: CORE (engine-owned: metadata, nodes, edges, schemas) plus zero-or-more provider-owned sections keyed by `ProviderTag` + `SubTag`. |

### Graph-blind WAL

`selene-persist` knows nothing about graph types. It receives a `&[Change]`
from `selene-graph` at commit time and a `RecoveryProvider` at recovery
time. Spec 04 defines the byte layout; the persistence crate validates
limits before handing decoded changes back to providers.

This separation has three consequences:

1. The WAL format can evolve independently of the graph data model.
2. Non-graph providers (vector indexes, future fulltext, future
   timeseries) can ride the same WAL by emitting their own `Change`
   variants without touching persistence code.
3. Recovery is a pure data pipeline: read snapshot, then replay the
   tail of the WAL.

### Two-step recovery

`selene_persist::recover(snapshot_dir, wal_dir, &ProviderRegistry) ->
RecoveryOutcome` does two things in order:

1. Locate the most recent valid `SLSN` snapshot (via `find_latest_snapshot`
   on a sorted directory listing), read its CORE sections to rebuild
   `SeleneGraph` state, and dispatch each provider-owned section to its
   `IndexProvider::read_section` callback.
2. Stream the WAL from the offset recorded in the snapshot footer,
   apply each `Change` to the rebuilt graph through the mutator, and
   notify providers via `on_change` for each replayed change.

The end state is byte-equivalent to the original graph plus the
provider-owned derived state. Snapshot frequency, sync policy
(`SyncPolicy::Always` | `SyncPolicy::Batched` | `SyncPolicy::Off`),
and compression thresholds are embedder-tunable knobs.

### Why graph-blind

If `selene-persist` knew the graph types directly, every schema change in
`selene-graph` would force a `selene-persist` recompile and risk a wire
format bump. By restricting the contract to "we serialize `&[Change]`",
the WAL becomes a stable cross-version protocol between graph and
durability.

---

## 5. Extension boundary

selene-db has exactly two extension points. Both are stable APIs.

### `IndexProvider` trait

Defined in `selene_graph::index_provider`, the trait shape is:

```text
trait IndexProvider: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn provider_tag(&self) -> ProviderTag;             // stable 4-byte ASCII
    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError>;
    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError>;
    fn on_change(&self, change: &Change) -> Result<(), ProviderError>;
    fn declared_sub_tags(&self) -> &[SubTag];
}
```

Provider authors:

- Reserve a 4-byte uppercase ASCII `ProviderTag` (first-party allocations
  include `VECT`, `FULL`, `TIMS`, `GRPR`).
- Implement interior mutability (`parking_lot::RwLock`, `papaya::HashMap`,
  etc.); the engine stores providers as `Arc<dyn IndexProvider>` and
  guarantees serialized calls per graph.
- Honor the re-entrancy contract: `on_change` must not initiate a write
  transaction on the same graph. Cross-thread re-entry that blocks the
  callback on a worker that itself calls `begin_write` deadlocks; this
  is documented misuse.

### Procedure packs

Defined in `selene-pack`, a procedure pack is a JSON-manifest-validated,
content-hashed bundle of named procedures registered into a frozen
`ProcedurePackRegistry` at construct time. The manifest is validated
against a JSON Schema 2020-12 schema with explicit gates
(`MANIFEST_LEVEL_GATES`, `PROCEDURE_LEVEL_GATES`,
`MANIFEST_VALIDATION_COVERAGE`, `FINAL_VALIDATION_COVERAGE`).

External packs implement the `ExternalProcedurePack` interface and supply
`ExternalGraphProcedure` (read-tier) or `ExternalMutationProcedure`
(write-tier) implementations. The registry tracks activation state through
a typestate state machine (`Uploaded -> Validating -> Staged -> Active`,
plus `Deprecated` and `Disabled` terminals). Activation transitions and
audit records are committed atomically with graph state through the same
mutation funnel (D18); there is no parallel ledger.

### Why algorithms and vectors live outside `selene-graph`

Decision D5 forbids non-graph capabilities from leaking into
`selene-graph`. The graph crate ships pure graph storage plus the
`IndexProvider` hook. Vector search lives in `selene-vector` and registers
through that hook; graph algorithms live in `selene-algorithms` and operate
over a frozen `GraphProjection`. The pack-adapter crates
(`selene-algorithms-pack`, `selene-vector-pack`) expose those capabilities
through GQL `CALL`. This split keeps the graph core honest: an embedder
who wants neither extension simply does not depend on those crates.

---

## 6. Architecture decisions D1-D21

The workspace shape is codified by twenty-one numbered decisions. They are
referenced from spec files and brief logs throughout the codebase.

### D1 — Library only

selene-db v1.0 is an embeddable Rust library. No server process, no
transport, no auth layer. Embedders own the network and policy surfaces.
This decision is what lets selene-db ship without a wire format claim
(ISO 39075 clause 4.2.3) and what makes `IW001` / `IW002` (principals and
authorization) the caller's responsibility.

### D2 — Strict ISO GQL parser

The query language is ISO/IEC 39075:2024 GQL. No Cypher, no SQL, no SPARQL
grammar in the parser. Constructs outside the v1.0 claimed feature register
are rejected by the Flagger at parse time. This eliminates a class of
"works on Neo4j but not on us" surprises and pins the language surface to
a stable external standard.

### D3 — Multi-graph workspace per process

A single process can host multiple `SharedGraph` instances side by side,
each with its own snapshot, write lock, WAL directory, and provider set.
Cross-graph transactions are out of v1.0 scope (feature `GT03` is not
claimed). This decision lets embedders run shard-per-graph or
tenant-per-graph patterns without process-level coordination.

### D4 — Snapshot isolation by ArcSwap publication

Readers see exactly the snapshot they captured via `SharedGraph::read()`.
Writers never mutate a snapshot in place; they construct a new
`Arc<SeleneGraph>` and atomically swap it. This is the durability of
`ArcSwap::store` plus the immutability of `imbl` collections.

### D5 — Non-graph capabilities in extension crates

Vectors, fulltext, time series, RDF, and graph algorithms ship as separate
crates outside `selene-graph`. They plug in through `IndexProvider` and the
procedure-pack registry. Refusing to widen the graph core is what keeps
the cold dependency closure for a graph-only embedder bounded.

### D6 — `Value` is a closed substitution union

`selene_core::Value` is a non-exhaustive enum with a canonical, append-only
variant order. Reordering variants is a major-version and durability-format
change. Extension types are surfaced via the `Value::Extended { type_id,
payload }` variant indexed by `ExtensionTypeId`, not by adding variants.

### D7 — Concurrency primitives

The concurrency stack is `ArcSwap` for snapshot publication, `parking_lot`
locks for writer mutual exclusion and provider state, `imbl` for
copy-on-write data structures, `RoaringBitmap` for label/id sets, and
`triomphe::Arc` for selected single-word reference counts. `std::sync`
locks are not used on the hot path because they poison on panic; an
in-process engine treats poison as a non-feature.

### D8 — Multi-crate workspace, no umbrella

`selene-core` is the leaf; `selene-persist` depends only on `selene-core`;
`selene-graph` builds on both; `selene-gql` and `selene-algorithms` depend
on `selene-core` + `selene-graph`; `selene-pack` is the widest, depending
on `selene-core` + `selene-persist` + `selene-graph` + `selene-gql`. There
is no `selene` umbrella crate that re-exports the others.

### D9 — Forbid unsafe

`#![forbid(unsafe_code)]` is set on every workspace crate. Performance
work happens through safe primitives (`SmallVec`, `roaring`, `imbl`,
`rkyv` zero-copy decode) or through battle-tested upstream crates. The
selene-db codebase contains no `unsafe` blocks of its own.

### D10 — `missing_docs = "deny"`

Every public item ships rustdoc. The lint is workspace-wide and CI-gated.
This is what makes the engine usable from the Rust docs without ever
touching the source.

### D11 — 700 LOC per-file cap

A static CI check enforces a 700-line cap on each tracked source file.
Files that bloat past 700 lines must be split. The cap is per-file only;
no per-crate or per-module budget gates merges.

### D12 — Conventional commits

Commits follow `type(scope): subject`. Scopes name the touched crate or
component (`feat(BRIEF-NN)`, `chore`, `docs`, etc.). Conventional
commits drive the changelog and the brief workflow audit trail.

### D13 — Postcard for WAL payloads

The WAL serializes `Vec<Change>` with postcard 1.x. Postcard is a tight
no-alloc-friendly format; selene-db does not hand-roll a binary format.
The WAL header and section table are framed by `selene-persist`.

### D14 — rkyv snapshots over sorted-vec intermediates

Snapshots are written by lowering `imbl` collections into sorted `Vec`
intermediates and archiving with rkyv 0.8 (`pointer_width_64`,
`unaligned`). Decoded archives are read zero-copy from a `Vec<u8>` buffer.
The `unaligned` feature lets rkyv decode out of 1-byte-aligned byte
buffers, which is what the snapshot reader produces.

### D15 — Procedure-pack manifest as JSON Schema

Procedure-pack manifests are validated against a JSON Schema 2020-12
schema with explicit gates. The schema is the contract; deviating manifests
are rejected at construction time, never at runtime.

### D16 — Frozen registry

Procedure-pack registration happens at registry construction. Once a
`ProcedurePackRegistry` is built, no packs can be added or removed.
This is what lets the analyzer and planner trust the registry without
locking on every lookup.

### D17 — Tiered procedure contexts

Procedures are partitioned by tier: read-tier (`GraphContext`),
write-tier (`MutationContext`), and procedure-tier (`ProcedureContext`).
Each tier has a concrete `Context` struct and a dyn-compatible `Procedure`
trait. The planner enforces tier compatibility against the surrounding
statement category at plan time.

### D18 — Lifecycle audit through the mutation funnel

Procedure-pack lifecycle events (`LifecycleEvent::Activated`,
`Deprecated`, `Disabled`, etc.) are emitted as `Change` variants through
the same `Mutator` that graph writes use. Audit records and graph writes
commit atomically; there is no parallel ledger and no audit-vs-graph
split-brain scenario.

### D19 — Blake3 for content hashing

Procedure-pack content hashes, snapshot section digests, and any other
"is this byte stream identical" checks use blake3. The hash function is
the same in every crate that needs one.

### D20 — Rustls-only TLS posture

Transitive dependencies must use rustls, never native-tls. CI enforces
this via `cargo-deny`. The engine itself ships no TLS code (per D1), but
its dependency closure cannot pull in OpenSSL bindings.

### D21 — Snapshot harness pattern

Every runtime surface that can drift (planner output, executor output,
procedure-pack signatures, algorithm output) is pinned by golden
`.snap` files. The pattern uses a pure-mirror DSL in `selene-testing`,
a renderer + integration test in the target crate, and `insta` for
snapshot management. See section 7 for the full pattern description.

---

## 7. Snapshot harness pattern (D21)

The snapshot harness exists because selene-db has many independent
producers of structured output that must not drift silently: planner
rewrites, executor row materialization, procedure-pack metadata
serialization, algorithm result shapes, vector-index section bytes,
recovery results. A change to any of these surfaces would otherwise hide
behind passing unit tests until an embedder noticed a wire-shape
difference.

### Pure-mirror DSL

`selene-testing` ships small read-only DSLs that mirror the public surface
of each pinned producer. The DSL crate does not depend on the target
crate; it expresses the shape of the producer's output as a set of
serializable structs.

For example, the planner mirror in `selene-testing` exposes a
`PlannerSummary` struct keyed by `PipelineOpId` with the op's kind, its
binding-table column shape, and any rule annotations the planner emits.
The target crate (`selene-gql`) implements a renderer that builds that
`PlannerSummary` from a real `ExecutionPlan`, and an integration test
fans out over a corpus of representative inputs, calling the renderer and
asserting against committed `.snap` files via `insta`.

### Why "pure-mirror"

The mirror crate's `[dependencies]` MUST NOT include the target crate. If
it did, the mirror would track every internal type change in the target
and the golden snapshots would re-render automatically on every refactor,
defeating the drift signal. Verifying the no-target-dep invariant is part
of the test corpus discipline (a `use <target>` grep in the mirror module
must be empty).

### Mirror drift defenses

For mirrors over `#[non_exhaustive]` foreign types, a count-only check
("the enum has 4 variants") is too weak. The pattern pins via three
mechanisms:

1. Use-imports: the mirror file lists every variant it cares about as a
   `use foreign::Enum::{A, B, C}` line; a removed variant fails to
   compile.
2. Anchor table: a `const ANCHOR: &[(name, count)]` table inside the
   mirror enumerates expected shape; a count-or-name mismatch fails a
   test.
3. Coverage test: a renderer test that exercises each anchor row;
   unexercised entries flag in CI.

### Snapshot files

Golden `.snap` files live alongside the integration tests that produced
them. `insta review` is the canonical workflow for accepting drift.

---

## 8. Engineering posture

### `#![forbid(unsafe_code)]`

The lint is set at workspace level and re-applied at each crate root.
Performance-sensitive code paths use safe primitives. The selene-db
codebase contains zero `unsafe` blocks. Donor patterns adapted from prior
forks are scrubbed of `unsafe` before integration; where a fast path
required `unsafe` in the donor, selene-db ships the safe equivalent or
declines the pattern.

### `missing_docs = "deny"`

Every `pub` item in every crate carries a rustdoc comment. The lint is
workspace-wide. Items that should not be documented because they should
not be public are made `pub(crate)` or moved to a non-public module.

### 700 LOC per-file cap

Each tracked source file must stay at or under 700 lines. The cap is
enforced by a CI gate. Files that approach the limit are refactored by
splitting modules, not by hoisting line breaks. The cap is per-file only;
brief acceptance bars never set per-crate budgets.

### Rustls-only TLS

The engine ships no TLS code, but the transitive dependency closure must
use rustls if it uses TLS at all. `cargo-deny` enforces a deny-list on
native-tls and openssl-sys. This decision keeps the engine portable
across platforms that lack a system OpenSSL.

### Conventional commits

Commits follow `type(scope): subject` with scopes naming the touched
component (`feat(BRIEF-NN)`, `chore`, `docs`, `refactor`, `test`).
Commit-message discipline is part of the changelog generation and the
audit trail across briefs.

### No hand-rolled crypto, TLS, async runtime, or serialization primitives

selene-db delegates to upstream crates: blake3 for hashing, xxhash for
non-cryptographic fingerprints, rkyv for archives, postcard for the WAL
payload, jiff for temporal types, rust_decimal for fixed-precision
decimal. The engine reserves the right to vendor a dependency if needed,
but does not reimplement these surfaces.

### Marathon mindset

Correctness, performance, and a stable extension contract come before
near-term shortcuts. Every PR ships with units, edge cases, error paths,
concurrency tests where state is shared, and property tests where
invariants are checkable.
