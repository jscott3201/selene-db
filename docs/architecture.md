# selene-db architecture

This document describes the current `c5c0a985` baseline. It assumes you have read
[`README.md`](../README.md) and want to understand the layering, concurrency
model, persistence design, and native procedure surface. The
[tracked 2.0 program](v2/README.md) owns the target architecture and finalized
decisions; future facade, catalog, profile, batch, and format-2 components are
not present in this baseline.

selene-db is an embeddable property graph engine for Rust that implements
selected ISO/IEC 39075:2024 GQL syntax and semantics. The current feature
register is implementation inventory, not a formal 2.0 conformance claim; see
the [evidence-gated policy](v2/conformance-policy.md). The engine is
library-only: no transport, no auth, no server.
Embedders depend on the published workspace crates or a local checkout and run
the engine in-process.

For operational detail on durability and recovery see
[`persistence-and-recovery.md`](persistence-and-recovery.md). For the GQL
surface see [`gql-reference.md`](gql-reference.md). For graph algorithm
surfaces see [`graph-algorithms.md`](graph-algorithms.md).

---

## 1. Crate dependency graph

At the c5 baseline, the workspace is a flat tree of six mandatory crates with
no umbrella facade. There are no opt-in extension crates. `selene-core` is the
leaf; every other crate transitively depends on it. The runtime dependency
direction is linear — `core → graph → algorithms → gql` — and
`selene-algorithms` never imports `selene-gql`. `selene-testing` is dev-only
and is consumed via `[dev-dependencies]`.

```text
   selene-core ──▶ selene-graph ──▶ selene-algorithms ──▶ selene-gql

   selene-persist ──▶ selene-core   (graph-blind durability; never sees Graph)

   (dev-only)  selene-testing  depends on selene-core, selene-graph
                                (+ selene-algorithms for fixtures)
```

| Crate | Depends on | Owns |
|---|---|---|
| `selene-core` | none | Foundation types: `Value` (mandatory ISO scalar values plus numeric, temporal, reference, byte-string, vector, JSON, list, record, path, and extension variants), `DbString`, `PropertyMap`, `LabelSet`, schema model, `Codec`, `Origin`, `Changeset`, GQLSTATUS table, ISO feature register. |
| `selene-graph` | core | In-memory property graph: ArcSwap + RwLock + immutable chunked persistent maps, `Mutator` write funnel, RoaringBitmap label / typed / composite indexes, `IndexProvider` / `DurableProvider` / `RecoveryProvider` hooks, `GraphTypeDef` runtime binding, `LiveIdSet` / `CompactionReport` / `compact_core` (CORE-internal densify compaction), `SharedGraph` + `WriteTxn`. |
| `selene-persist` | core | Graph-blind WAL (`SLDB` magic) + rkyv-archived snapshots (`SLSN`, TLV-tagged sections) + recovery + the append-only `audit.log` (`SLAU`). Never sees `Graph` — takes `&[Change]`, returns `RecoveryResult`. |
| `selene-algorithms` | core, graph | `GraphProjection` + `ProjectionCatalog` foundation, 19 public algorithm surfaces (structural / pathfinding / centrality / community), and the native Rust API (free functions + the `GraphAlgorithms` extension trait — a methods-on-graph convenience, with the 1024-thread `Parallelism` cap) + the snapshot harness. Independent of `selene-gql`. |
| `selene-gql` | core, graph, algorithms | Pest GQL grammar, AST, semantic analyzer, planner, rule-based optimizer, row-at-a-time executor, Flagger, the `ProcedureRegistry` trait, and its sole frozen production impl `BuiltinProcedureRegistry` — 50 `selene.*` platform built-ins plus 19 `algo.*` procedures binding `CALL` directly over native engine APIs. |
| `selene-testing` | core, graph (+ algorithms for fixtures) | Shared fixtures, synthetic graph generators, pure-mirror snapshot-harness DSLs for the planner / executor / algorithm corpora. Consumed via `[dev-dependencies]`. |

The dependency graph is intentionally acyclic with a single sink
(`selene-core`) and a single broad runtime consumer (`selene-gql`).
`selene-algorithms` is a mandatory first-class crate that sits between
`selene-graph` and `selene-gql`: it owns the native algorithm API and never
imports the GQL layer, so the graph storage core stays free of the
query/procedure surface.

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
are the only entry points from raw GQL. The parser is a pest 2.9 PEG defined
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
phase: a construct outside the current implementation register is rejected with
a `GqlStatus` flag. That register does not establish a formal claim.

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
`selene-persist` for WAL append. Engine-owned audit events route through the
same funnel into a dedicated append-only `audit.log` substrate (`SLAU`),
WAL-first then audit-after, so there is no parallel ledger and no
audit-vs-graph split-brain.

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
read path.

| Primitive | Where it is used | Why |
|---|---|---|
| `arc_swap::ArcSwap` | `SharedGraph` publishes each committed snapshot as `Arc<SeleneGraph>`. | Readers grab a pointer in a single relaxed load. No lock acquisition, no allocation, no poisoning. |
| `parking_lot::RwLock` | Per-graph write lock; also wraps mutable index state inside providers. | parking_lot is non-reentrant, never poisons, and is materially faster than `std::sync::RwLock`. Lock poisoning is a non-feature for an in-process embedded engine: a panic during commit must abort the process, not leave readers staring at a `PoisonError`. |
| `immutable_chunkmap::MapM` | Copy-on-write ID, label, and adjacency maps inside `SeleneGraph`. | The Arc-backed chunked tree keeps clone O(1) and path-copies mutations, so a writer can publish without copying the entire map. |
| `roaring::RoaringBitmap` | Label indexes, deleted-id sets, candidate selections. | Compact, cache-friendly representation for sparse `NodeId` sets. |
| `triomphe::Arc` | Property-value reference counting in selected hot paths. | `triomphe::Arc` is non-`Weak`, single-word, and avoids the `std::sync::Arc` weak-count overhead where weak refs are not used. |

### How a write commits

1. The embedder calls `SharedGraph::begin_write`, which acquires the write
   `RwLock`. Only one writer can hold this lock per graph.
2. The returned `WriteTxn<'g>` exposes a `Mutator` whose methods append
   `Change`s into a transaction-local buffer.
3. On `WriteTxn::commit` (or `commit_with_principal`), the **seal** half runs
   on the calling thread under the held lock: the mutator clones the current
   snapshot's persistent data structures, applies the `Change` list, and
   constructs a new `Arc<SeleneGraph>`. `seal` then **releases the write
   lock** and hands an owned, `Send` bundle to the committer.
4. The **publish tail** runs on the single per-graph committer thread, which is
   the sole writer of the `ArcSwap` cell. It stamps the HLC and **appends** the
   entry to the WAL.
5. One **group flush** (`fsync`) covers the whole contiguous run. This is the
   durability barrier.
6. Only then is the new snapshot **published** via `ArcSwap::store`. Readers in
   flight continue against the old snapshot until they drop their `Arc`; new
   readers see the new snapshot on their next `read()`.
7. Registered `IndexProvider`s are notified via `on_change(&Change)`. The
   re-entry contract (see `IndexProvider` rustdoc) forbids same-graph
   `begin_write` calls from inside `on_change`.
8. The commit is **acknowledged** — `commit()` returns `Ok(CommitOutcome)`.

The ordering in steps 4-6 is load-bearing: the fsync precedes publication, so
no reader can observe a commit that is not already durable, and no caller is
told "committed" before the barrier. That barrier is the R1 fsync-before-publish
guarantee introduced in v1.2 (BRIEF 2); this document described the earlier
publish-then-append order until it was corrected. See
`docs/persistence-and-recovery.md` for the outcome contract, including why a
failure past step 4 reports an *indeterminate* outcome rather than a rollback.

### How a reader holds a snapshot

A read path inside the executor calls `SharedGraph::read()` once, receives
an `Arc<SeleneGraph>`, and holds that `Arc` for the duration of the
statement. No further coordination is needed: the snapshot is immutable, and
the writer cannot mutate the structures the reader sees.

### Why locks don't poison

selene-db never propagates a `LockResult` to user code, and parking_lot locks
do not poison on panic. Readers cannot observe an in-progress write because
they never look at the writer's transaction buffer; they only observe the last
fully-published `ArcSwap` cell.

A panic on the committer thread does not abort the process. It is caught at the
committer's `catch_unwind` boundary, which **poisons the engine**: the panicking
commit and every later one fail with `GraphError::IndeterminateOutcome`, and the
graph must be reopened through recovery. That is a different mechanism from lock
poisoning — the locks are fine; the *engine* is the thing declared unusable,
because a panic between seal and publish can leave the live graph and the
published snapshot divergent with no in-process way to reconcile them.

---

## 4. Persistence model

selene-db separates persistence into two independent file formats, each
with a stable 4-byte magic prefix.

| Format | Magic | Producer | Consumer | Purpose |
|---|---|---|---|---|
| WAL entry | `SLDB` | `selene_persist::WalWriter` | `WalReader`, `recover` | Framed entry: header (length, principal, flags, checksum) followed by postcard-serialized `Vec<Change>`. Ordinary frames are logical commits; typed empty checkpoint-watermark frames reserve physical snapshot epochs without advancing graph generation. |
| Snapshot | `SLSN` | `SnapshotBuilder` | `SnapshotReader` | rkyv-archived snapshot with TLV-tagged sections: CORE (engine-owned: metadata, nodes, edges, schemas) plus zero-or-more provider-owned sections keyed by `ProviderTag` + `SubTag`. |

### Graph-blind WAL

`selene-persist` knows nothing about graph types. It receives a `&[Change]`
from `selene-graph` at commit time and a `RecoveryProvider` at recovery
time. Spec 04 defines the byte layout; the persistence crate validates
limits before handing decoded changes back to providers.

This separation has three consequences:

1. The WAL format can evolve independently of the graph data model.
2. First-party derived-state providers (the `IndexProvider` /
   `DurableProvider` / `RecoveryProvider` plumbing) ride the same WAL by
   emitting their own `Change` variants without touching persistence code.
3. Recovery is a pure data pipeline: read snapshot, then replay the
   tail of the WAL.

### Two-step recovery

`selene_persist::recover(data_dir, &ProviderRegistry) -> RecoveryOutcome`
acquires a shared `PersistenceReadGuard`, then does two things in order:

1. Select the MANIFEST-authoritative `SLSN` snapshot (or the highest legacy
   snapshot when no MANIFEST exists), verify it, and dispatch its sections to
   `RecoveryProvider::read_section`.
2. Stream post-snapshot logical commit frames from the active WAL and notify
   providers through `on_changes`; physical checkpoint-watermark frames are
   skipped.

The shared guard pins one snapshot/WAL epoch against rotation and prune for the
entire replay. `SharedGraph::recover` acquires an existing WAL writer first and
retains it through guarded replay. For a snapshot-only directory, it verifies
recovery under the guard before creating a seeded WAL with a non-blocking open.
In both paths the retained writer tip must exactly match the replay high-water.

### Why graph-blind

If `selene-persist` knew the graph types directly, every schema change in
`selene-graph` would force a `selene-persist` recompile and risk a wire
format bump. By restricting the contract to "we serialize `&[Change]`",
the WAL becomes a stable cross-version protocol between graph and
durability.

---

## 5. Internal seams

selene-db is a single native graph engine: there is no loadable-extension or
procedure-pack system, and there are no third-party plug-in points. The
internal seams below are load-bearing *engine* architecture — they keep the
durable-state and plan/execute boundaries clean — not a public extension API.

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

The provider plumbing (first-party only):

- Reserves a 4-byte uppercase ASCII `ProviderTag` so its snapshot sections
  are addressable independently of the CORE sections.
- Implements interior mutability (`parking_lot::RwLock`, etc.); the engine
  stores providers as `Arc<dyn IndexProvider>` and guarantees serialized
  calls per graph.
- Honors the re-entrancy contract: `on_change` must not initiate a write
  transaction on the same graph. Cross-thread re-entry that blocks the
  callback on a worker that itself calls `begin_write` deadlocks; this
  is documented misuse.

### `ProcedureRegistry` trait

`CALL` dispatch is mediated by the `ProcedureRegistry` trait, defined
in `selene-gql`. It is the plan/execute seam: the planner resolves procedure
signatures and the executor dispatches calls through a `&dyn
ProcedureRegistry`. The trait has exactly one frozen production
implementation — the concrete native `BuiltinProcedureRegistry`
(`selene-gql/src/runtime/builtin_registry.rs`) — which registers 69
procedures at construction (the 50 `selene.*` platform built-ins plus the 19
`algo.*` procedures) and reports a constant `registry_version()` of `0` so
the CALL plan cache never invalidates. The injectable `&dyn` seam exists for
the test harness, not for third-party packs: there is no loadable-pack
apparatus, no manifest validation, and no activation lifecycle.

### Why algorithms live outside `selene-graph`

Graph storage and graph algorithms are kept in separate crates so the
storage core never grows the algorithm surface. `selene-graph` ships pure
graph storage plus the `IndexProvider` hook; the algorithms live in the
mandatory `selene-algorithms` crate and operate over a frozen
`GraphProjection`. `selene-gql`'s `BuiltinProcedureRegistry` binds `CALL
algo.*` directly over the native `selene-algorithms` Rust API — no adapter
crate, no `CALL`-grammar extension (the ISO `CALL` surface is unchanged per
IW010). The algorithm crate never imports `selene-gql`, which keeps the
`core → graph → algorithms → gql` direction acyclic.

---

## 6. Current implementation choices

Earlier documents used local numbered labels for the c5 implementation
inventory. Those labels are not 2.0 decision authority. The canonical program
uses [D-001 through D-022](v2/decisions/finalized.md).

### Library only

selene-db is an embeddable Rust library. No server process, no transport, no
auth layer. Embedders own the network and policy surfaces. This decision is
what lets selene-db ship without a wire format claim (ISO 39075 clause 4.2.3)
and what makes `IW001` / `IW002` (principals and authorization) the caller's
responsibility.

### Strict ISO GQL parser

The query language is ISO/IEC 39075:2024 GQL. No Cypher, no SQL, no SPARQL
grammar in the parser. Constructs outside the current implementation register are
rejected by the Flagger at parse time. This eliminates a class of
"works on Neo4j but not on us" surprises and pins the language surface to
a stable external standard.

### Multi-graph workspace per process

A single process can host multiple `SharedGraph` instances side by side,
each with its own snapshot, write lock, WAL directory, and provider set.
Cross-graph transactions are outside the current runtime inventory (feature
`GT03` is not runtime-supported). This decision lets embedders run shard-per-graph or
tenant-per-graph patterns without process-level coordination.

### Snapshot isolation by ArcSwap publication

Readers see exactly the snapshot they captured via `SharedGraph::read()`.
Writers never mutate a snapshot in place; they construct a new
`Arc<SeleneGraph>` and atomically swap it. This is the durability of
`ArcSwap::store` plus structurally shared persistent maps and Arc-backed row
storage.

### Native capability ownership

selene-db is one cohesive native graph engine. Graph algorithms ship in the
mandatory `selene-algorithms` crate outside `selene-graph`; vectors, text, JSON,
indexes, and native procedures are in-tree engine features owned by their
current crates. Time-series, RDF, and GraphRAG-specific policy are not current
engine surfaces.

### `Value` is a closed substitution union

`selene_core::Value` is a non-exhaustive enum with a canonical, append-only
variant order. Reordering variants is a major-version and durability-format
change. Extension types are surfaced via the `Value::Extended { type_id,
payload }` variant indexed by `ExtensionTypeId`, not by adding variants.

### Concurrency primitives

The concurrency stack is `ArcSwap` for snapshot publication, `parking_lot`
locks for writer mutual exclusion and provider state, `immutable-chunkmap` for
copy-on-write maps, `RoaringBitmap` for label/id sets, and
`triomphe::Arc` for selected single-word reference counts. `std::sync`
locks are not used on the hot path because they poison on panic; an
in-process engine treats poison as a non-feature.

### Multi-crate workspace, no umbrella

`selene-core` is the leaf; `selene-persist` depends only on `selene-core`;
`selene-graph` builds on `selene-core`; `selene-algorithms` builds on
`selene-core` + `selene-graph`; `selene-gql` is the widest runtime crate,
depending on `selene-core` + `selene-graph` + `selene-algorithms`. The
runtime direction is the linear chain `core → graph → algorithms → gql`, and
`selene-algorithms` never imports `selene-gql`. There is no `selene` umbrella
crate that re-exports the others.

### Forbid unsafe

`#![forbid(unsafe_code)]` is set on every workspace crate. Performance work
happens through safe upstream APIs (`SmallVec`, `roaring`,
`immutable-chunkmap`, `rkyv` zero-copy decode). The selene-db codebase contains
no `unsafe` blocks of its own.

### `missing_docs = "deny"`

Every public item ships rustdoc. The lint is workspace-wide and CI-gated.
This is what makes the engine usable from the Rust docs without ever
touching the source.

### 700 LOC per-file cap

A static CI check enforces a 700-line cap on each tracked source file.
Files that bloat past 700 lines must be split. The cap is per-file only;
no per-crate or per-module budget gates merges.

### Conventional commits

Commits follow `type(scope): subject`. Scopes name the touched crate or
component (`feat(BRIEF-NN)`, `chore`, `docs`, etc.). Conventional
commits drive the changelog and the brief workflow audit trail.

### Postcard for WAL payloads

The WAL serializes `Vec<Change>` with postcard 1.x. Postcard is a tight
no-alloc-friendly format; selene-db does not hand-roll a binary format.
The WAL header and section table are framed by `selene-persist`.

### rkyv snapshots over sorted-vec intermediates

Snapshots are written by lowering persistent engine maps into sorted `Vec`
intermediates and archiving with rkyv 0.8 (`pointer_width_64`,
`unaligned`). Decoded archives are read zero-copy from a `Vec<u8>` buffer.
The `unaligned` feature lets rkyv decode out of 1-byte-aligned byte
buffers, which is what the snapshot reader produces.

### Frozen native procedure registry

`CALL` is dispatched through the `ProcedureRegistry` trait, whose sole
production implementation, `BuiltinProcedureRegistry`, registers its full
procedure set (69 procedures: 50 `selene.*` platform built-ins plus 19 `algo.*`
procedures) once at construction. The registry is frozen — nothing is added
or removed after construction, and `registry_version()` is a constant `0` —
which lets the analyzer and planner trust the registry and keep the CALL plan
cache stable without locking on every lookup.

### Tiered procedure contexts

Procedures are partitioned by tier: read-tier (`GraphContext`),
write-tier (`MutationContext`), and procedure-tier (`ProcedureContext`).
Each tier has a concrete `Context` struct and a dyn-compatible `Procedure`
trait. The planner enforces tier compatibility against the surrounding
statement category at plan time, and the registry re-checks the tier on
dispatch so a read-only built-in can never re-enter the write funnel.

### Engine-owned audit through the mutation funnel

Engine-owned audit events route through the same `Mutator` that graph writes
use, into a dedicated append-only `audit.log` substrate (`SLAU`) with
retention independent of the WAL/snapshot lineage. Writes are WAL-first then
audit-after through the one funnel, so there is no parallel ledger and no
audit-vs-graph split-brain scenario.

### Blake3 for content hashing

Snapshot section digests and any other "is this byte stream identical"
checks use blake3. The hash function is the same in every crate that needs
one.

### Rustls-only TLS posture

Transitive dependencies must use rustls, never native-tls. CI enforces
this via `cargo-deny`. The engine itself ships no TLS code, but
its dependency closure cannot pull in OpenSSL bindings.

### Snapshot harness pattern

Every runtime surface that can drift (planner output, executor output,
procedure signatures, algorithm output) is pinned by golden
`.snap` files. The pattern uses a pure-mirror DSL in `selene-testing`,
a renderer + integration test in the target crate, and `insta` for
snapshot management. See section 7 for the full pattern description.

---

## 7. Snapshot harness pattern

The snapshot harness exists because selene-db has many independent
producers of structured output that must not drift silently: planner
rewrites, executor row materialization, procedure-signature metadata
serialization, algorithm result shapes, recovery results. A change to
any of these surfaces would otherwise hide
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

Correctness, performance, and a cohesive native engine come before
near-term shortcuts. Every PR ships with units, edge cases, error paths,
concurrency tests where state is shared, and property tests where
invariants are checkable.
