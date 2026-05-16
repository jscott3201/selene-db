# Persistence and recovery

This document describes how `selene-db` durably persists its in-memory property
graph and how the engine reconstructs that state after a restart. It is aimed
at engineers operating `selene-db` in production: configuring durability,
sizing recovery windows, and taking backups.

The persistence subsystem lives entirely inside the
[`selene-persist`](../crates/selene-persist) crate. It owns two on-disk
formats — a write-ahead log and a snapshot envelope — and a recovery
orchestrator that drives them. It depends only on `selene-core`, never on the
graph types, by design.

## The two layers

`selene-db` writes to two complementary durability artifacts in the same
data directory:

| Artifact      | Magic  | Purpose                                           | Cost model                                |
| :------------ | :----- | :------------------------------------------------ | :---------------------------------------- |
| Write-ahead log | `SLDB` | One append per committed transaction.              | Constant per commit; bounded by `fsync`.  |
| Snapshot      | `SLSN` | Point-in-time freeze of the entire materialized graph. | Linear in graph size; taken on a cadence. |

The WAL gives the engine **per-commit durability**: every `Mutator::commit`
that returns success has produced bytes that are reachable after a process
crash. The snapshot gives the engine **bounded recovery time**: replaying a
multi-million-entry WAL on every restart would be unacceptable, so the engine
periodically materializes the live graph as a single archive and trims its
"replay from" watermark.

Recovery is always the same two steps in order:

1. Apply the most recent snapshot (if any).
2. Replay every WAL entry whose sequence is greater than the snapshot's
   sequence.

This is the only ordering the engine supports. There is no incremental
snapshot format, no delta snapshot, no shadow paging.

## The graph-blind boundary

`selene-persist` does not depend on `selene-graph`. It never sees a
`SeleneGraph`, a `NodeId`-keyed property store, or a typed index. Its job
ends at decoding bytes and producing one of two callbacks:

- A snapshot **section payload**, identified by a `(provider, sub)` four-byte
  tag pair.
- A WAL **`Change`**, which is the canonical mutation enum in
  `selene-core` ([`crates/selene-core/src/changeset.rs`](../crates/selene-core/src/changeset.rs),
  `pub enum Change`).

The contract is the [`RecoveryProvider`](../crates/selene-persist/src/provider.rs)
trait:

```rust
pub trait RecoveryProvider: Send + Sync {
    fn provider_tag(&self) -> [u8; 4];
    fn read_section(&self, sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()>;
    fn on_change(&self, change: &Change) -> RecoveryResult<()>;
}
```

A `ProviderRegistry` maps four-byte provider tags to `Arc<dyn RecoveryProvider>`
implementations. During recovery the orchestrator:

- Routes each snapshot section to the provider whose tag matches the
  section's `provider` field.
- Fans every replayed `Change` out to **every** registered provider, in
  deterministic provider-tag order. Each provider decides whether the change
  affects its state.

This boundary is load-bearing. It means the persistence layer does not have
to be re-released when graph types evolve. It also means that **extensions
with persistent state** (a vector index, a custom index family, a procedure
pack with audit storage) plug in by registering their own `RecoveryProvider`
implementation. See [Recovery semantics for extensions](#recovery-semantics-for-extensions).

The core graph state participates exactly the same way: `selene-graph` ships
the `CoreProvider` (provider tag `CORE`) that reconstructs the in-memory
graph from `CORE/*` snapshot sections and post-snapshot `Change`s.

## Writing — the WAL

Every committed `Mutator` operation in `selene-graph` produces a `Vec<Change>`.
The persistence layer appends this slice to the WAL as a single entry. Each
entry carries:

- A monotonic 64-bit sequence number assigned by the writer.
- An HLC timestamp (`HlcTimestamp`).
- An origin (`Origin::Local`, or `Origin::Replicated { source_node_id, source_seq }`).
- An optional principal (audit-trail actor; capped at 254 bytes).
- The encoded `Change` payload, with an xxh3 checksum and an optional
  zstd-compressed body when the encoded payload crosses the compression
  threshold.

Append is single-threaded. The `WalWriter` holds an exclusive OS-level
advisory lock on the WAL file for its entire lifetime; a second
`WalWriter::open` on the same path returns
[`PersistError::WriterLockHeld`](../crates/selene-persist/src/error.rs).

### `SyncPolicy`

Durability versus throughput is controlled by the `SyncPolicy` enum in
[`writer.rs`](../crates/selene-persist/src/writer.rs):

```rust
pub enum SyncPolicy {
    /// Flush and fsync after every `N` appended entries, on explicit flush,
    /// and when the writer is dropped. `EveryN(1)` is durability-by-default.
    /// Values > 1 opt into group commit. `EveryN(0)` normalizes to `EveryN(1)`.
    EveryN(u32),
    /// Never fsync during append or drop; only explicit `WalWriter::flush`
    /// fsyncs. Explicit opt-in for benchmark parity and offline paths.
    OnFlushOnly,
}
```

The two policies and their semantics:

| Policy                | Semantics                                                                                | Use case                                              |
| :-------------------- | :--------------------------------------------------------------------------------------- | :---------------------------------------------------- |
| `EveryN(1)`           | fsync after every appended entry; fsync on flush; fsync on drop.                          | Server workloads; the default; "durable per commit".  |
| `EveryN(N)`, `N > 1`  | fsync after every `N` appended entries; fsync on flush; fsync on drop.                    | Group commit; trades up to `N-1` entries of recent durability for throughput. |
| `OnFlushOnly`         | No fsync on append. No fsync on drop. Only explicit `WalWriter::flush` syncs.             | Benchmark parity, offline import. **Not** for production. |

The default `WalConfig::default()` is `SyncPolicy::EveryN(1)` — durability
per commit. Choose `EveryN(N)` only after measuring; the WAL append
benchmarks in [BENCHMARKS.md](../BENCHMARKS.md) show batched appends already
amortize fsync cost.

### `WalConfig`

```rust
pub struct WalConfig {
    pub sync_policy: SyncPolicy,
    /// Highest WAL sequence covered by the snapshot this file extends.
    pub snapshot_seq: u64,
}
```

`snapshot_seq` is the watermark a snapshot publishes; it is written into the
WAL file header on a fresh file, and used to seed the first appended
sequence so it lands at `snapshot_seq + 1`. On reopen, the on-disk header
wins — recovery never moves a snapshot watermark backward, even if the
config passed in is stale.

### Append flow

`WalWriter::append(hlc, origin, principal, &changes)` is the only mutation
entry point. On any error during append, the in-memory sequence counter is
**not** advanced and the file is truncated back to the last fully-committed
entry. The next append (or a reopen + retry) observes a consistent state.

## Snapshot creation

A snapshot is an `rkyv`-archived envelope of TLV-tagged sections. The file
header (32 bytes) carries:

- 4-byte magic `SLSN`.
- 2-byte major version (currently `1`), 2-byte minor version (currently `0`).
- 2-byte flags (per-section compression toggle in v1.0).
- 2-byte section count.
- 4 reserved bytes (must be zero).
- 16-byte blake3-128 hash of the section table plus payload bytes.

The header is followed by a fixed-row section table (24 bytes per row,
provider tag + sub-tag + payload offset + payload length) and the
concatenated section payloads.

Construction lives in
[`SnapshotBuilder`](../crates/selene-persist/src/snapshot_writer.rs):

```rust
use selene_persist::{SectionCompression, SnapshotBuilder, SnapshotConfig};

let mut builder = SnapshotBuilder::new(SnapshotConfig {
    dir: data_dir.to_path_buf(),
    sequence: snapshot_seq,
    compression: SectionCompression::PerSection { level: 1 },
    fsync: true,
});
builder.add_section(*b"CORE", *b"META", core_meta_bytes)?;
builder.add_section(*b"CORE", *b"NODE", core_node_bytes)?;
builder.add_section(*b"VECT", *b"GRPH", hnsw_topology_bytes)?;
let path = builder.finalize()?;
```

`finalize()` writes `snapshot.{sequence}.snap.tmp`, fsyncs it when
`fsync: true`, then atomically **hard-links** it to
`snapshot.{sequence}.snap` and removes the tmp. The hard-link is the
race-safe alternative to `rename` (which silently overwrites on POSIX); a
sequence collision fails fast with `Io(AlreadyExists)`.

Each section's `(provider, sub)` tag pair identifies its producer. Common
tags shipped by the workspace:

| Provider | Sub     | Producer                                  |
| :------- | :------ | :---------------------------------------- |
| `CORE`   | various | `selene-graph` core state (`CoreProvider`).|
| `VECT`   | `GRPH`  | `selene-vector` HNSW topology.            |
| `VECT`   | `VECS`  | `selene-vector` f32 vector payload.       |
| `VECT`   | `QUNT`  | `selene-vector` quantized payload.        |
| `IVFP`   | `CQNT`  | `selene-vector` IVF coarse quantizer.     |
| `IVFP`   | `IPQB`  | `selene-vector` IVF residual PQ codebook. |
| `IVFP`   | `POST`  | `selene-vector` IVF posting lists.        |

Extension authors pick a four-byte ASCII `provider` tag and a `sub` tag per
section family they need. Tags are advisory but must be globally unique
within a registry — duplicate registration fails with
`PersistError::DuplicateProviderTag`.

### When to snapshot

The engine does not take snapshots automatically; embedders drive cadence
based on workload and recovery-time budget. A reasonable starting policy:

- Take a snapshot after every `N` WAL entries (e.g., `N = 1_000_000`).
- Take a snapshot after every `T` minutes of wall-clock (e.g., hourly).
- Take a snapshot before a planned shutdown.

After a snapshot is finalized, the old WAL can be archived or pruned. The
in-process protocol is:

1. Take a consistent read of the graph (`SharedGraph` clones the snapshot via
   `ArcSwap`, so this is lock-free).
2. Build the snapshot at `sequence = wal.last_sequence()`.
3. Atomically rotate the WAL: open a new file with
   `WalConfig { snapshot_seq: sequence, .. }`, the old WAL can be removed
   once the new snapshot is durable on disk.

## Two-step recovery

On startup, the persistence layer runs
[`recover(dir, &registry)`](../crates/selene-persist/src/recovery.rs):

```rust
use selene_persist::{ProviderRegistry, recover};

let mut registry = ProviderRegistry::new();
registry.register(core_provider.clone())?;
registry.register(vector_provider.clone())?;

let outcome = recover(data_dir, &registry)?;
```

The orchestrator:

1. Scans `data_dir` for `snapshot.{seq}.snap` files and picks the one with
   the highest sequence.
2. Verifies the snapshot's body hash. On mismatch this is a **hard failure**
   (`PersistError::BodyHashMismatch`); recovery does not silently fall back to
   an older snapshot.
3. For each section in the snapshot's section table (in declared order),
   looks up the provider by tag and calls `provider.read_section(sub, bytes)`.
   Unknown provider tags surface as `PersistError::UnknownProvider`.
4. Opens the WAL (`wal.log`) if present. The WAL header's `snapshot_seq` must
   match the snapshot just applied; mismatch is rejected with
   `PersistError::WalSnapshotMismatch`.
5. Iterates WAL entries whose `sequence > snapshot_seq` and fans every
   decoded `Change` out to every registered provider. The order is
   deterministic provider-tag ascending order.
6. Returns a `RecoveryOutcome` with the applied snapshot sequence, the last
   WAL sequence, the set of providers invoked, and counters for changes
   applied versus replicated changes deduplicated.

For applications that bundle the graph plus extension providers, the
convenience wrapper in `selene-graph` does the registration:

```rust
use selene_graph::SharedGraph;
use selene_core::GraphId;

let graph = SharedGraph::recover(data_dir, GraphId::new(1))?;
```

`SharedGraph::recover` constructs a `CoreProvider`, registers it, drives
`selene_persist::recover`, then materializes the rebuilt graph state. The
closed-graph variant is `SharedGraph::recover_closed(dir, graph_id, bound_type)`.

A truncated WAL tail (torn write at the end of the log) is **not** an error
during recovery: the iterator stops at the last fully-checksummed entry,
matching the on-open scan that `WalWriter` runs.

## Recovery semantics for extensions

Extensions with persistent state participate via their `RecoveryProvider`
implementation. The contract has two surfaces:

- `read_section(sub, bytes)`: called once per snapshot section that matches
  this provider's tag, in the section table's declared order. The provider
  decodes the bytes back into its in-memory state.
- `on_change(change)`: called for every WAL `Change` past the snapshot
  sequence. The provider decides whether the change is relevant to it (most
  ignore most changes).

The recovery boundary has a sharp edge: **WAL replay only covers events
after the snapshot's last sequence**. Any provider that maintains
pre-training or staging state must persist that state in the snapshot
itself, not rely on WAL replay to rebuild it. A staged provider that emits
an empty snapshot section and assumes the WAL will reconstruct the staging
buffer will silently lose that buffer on recovery, because the pre-snapshot
WAL entries have already been pruned.

The discipline is: if your provider can be in a state that affects future
behavior but is not derivable from post-snapshot `Change`s alone, capture
that state in a snapshot section. This applies to IVF training centroids,
HNSW build parameters captured at construction time, OPQ rotation matrices,
and any other "computed once, used many times" artifact.

## Snapshot versioning

The snapshot format carries explicit `version_major` and `version_minor` in
the header. The `selene-persist` crate maintains the following discipline:

- **Read-side compatibility for prior versions** is byte-for-byte preserved
  across the v1.x major version. Sections that decoded on an earlier minor
  version continue to decode on later minor versions in the same major.
- **Writers always emit the current version**. There is no flag to write a
  prior format.
- An unsupported major version surfaces as `PersistError::UnsupportedVersion`
  at open time. There is no automatic format upgrade pass; embedders that
  cross a major version boundary go through an explicit migration step.

Reserved bytes in the snapshot header (offsets 12–15) must be zero on disk;
nonzero values are rejected as `ReservedBytesNonZero` to make accidental
forward-compatibility hacks visible.

Per-section payload format is owned by each section's producer. The
`provider`/`sub` tag pair identifies which decoder runs; the producer is
responsible for tagging its own byte layouts with versions if it needs to
evolve them. The vector index extension does this through subsection
version bytes inside its rkyv-archived bodies; first-party CORE sections do
the same.

## Backups

A consistent backup is **one** snapshot plus the WAL it extends, that is:

- `snapshot.{S}.snap` for some sequence `S`, and
- `wal.log` whose header `snapshot_seq` equals `S`.

The recipe:

1. Quiesce writes (close all `WalWriter` instances) or take the backup
   between scheduled snapshots while writes continue — the snapshot file is
   atomically published, so a half-written `snapshot.{seq}.snap.tmp` is
   never visible.
2. Take a snapshot. `SnapshotBuilder::finalize()` is atomic; the final path
   appears in one filesystem operation.
3. Copy `snapshot.{seq}.snap` and the current `wal.log` to the backup
   destination.

To restore: drop the two files into the configured data directory and start
up. `recover` will apply the snapshot, replay the WAL forward, and the
engine will reach the same logical state.

For point-in-time restore you can keep a chain of WAL files (one per
snapshot rotation) plus all snapshots, and replay forward from any snapshot.
The format is forward-compatible across snapshots that share a sequence
chain, but the engine itself does not ship WAL archival or pruning tooling;
the embedder owns rotation.

## What can go wrong

The persistence layer is engineered to make every failure mode either
recoverable or loud, never silent. The expected failure modes:

| Failure                                | Detection                                  | Outcome                                                |
| :------------------------------------- | :----------------------------------------- | :----------------------------------------------------- |
| Torn write at WAL tail                 | Per-entry xxh3 checksum mismatch, oversized payload/principal lengths, or truncated body. | The on-open scan in `WalWriter::open` truncates the file to the last fully-committed entry. Logged at WARN. |
| Snapshot file truncated                | Header short read.                         | `PersistError::TruncatedSnapshotHeader`; recovery refuses to fall back silently. |
| Snapshot body corruption               | blake3-128 body hash mismatch.             | `PersistError::BodyHashMismatch`. Hard failure — the snapshot is unusable. |
| Unsupported snapshot or WAL version    | Magic + version check on open.             | `PersistError::UnsupportedVersion`. The embedder must run a migration. |
| Reserved bytes set in snapshot header  | Read-time reserved-byte audit.             | `PersistError::ReservedBytesNonZero`.                  |
| Duplicate provider tag in registry     | `ProviderRegistry::register`.              | `PersistError::DuplicateProviderTag` at startup.       |
| Snapshot section for unknown provider  | Recovery routes by tag.                    | `PersistError::UnknownProvider`. The embedder forgot to register a provider. |
| WAL/snapshot epoch mismatch            | WAL header `snapshot_seq` vs applied snapshot. | `PersistError::WalSnapshotMismatch`. The pair on disk is inconsistent. |
| Non-monotonic WAL sequence             | Per-entry header check during scan.        | `PersistError::NonMonotonicSequence`. Indicates the WAL was edited or merged incorrectly. |
| Schema drift (closed graph)            | `CoreProvider` validates declared `GraphType` against the snapshot's bound type. | `GraphError::Provider` at recovery time. |

The persistence layer's posture is: a recoverable failure (torn tail)
recovers transparently; an unrecoverable failure (body hash mismatch)
refuses to start, so a stale or corrupt artifact does not silently degrade
the engine's state.

## Configuration recipes

Three concrete configurations that cover common deployment shapes.

### Workstation / development

Goal: fast iteration; some data loss on crash is acceptable; recovery time
matters less than throughput.

```rust
use selene_persist::{SyncPolicy, WalConfig};

let wal_config = WalConfig {
    sync_policy: SyncPolicy::EveryN(64), // group-commit every 64 entries
    snapshot_seq: 0,
};
// Take a snapshot at end of session, or every 5 minutes of wall-clock.
```

### Embedded edge

Goal: durable per commit; recovery from WAL only (no snapshots); minimal
storage footprint.

```rust
use selene_persist::{SyncPolicy, WalConfig};

let wal_config = WalConfig {
    sync_policy: SyncPolicy::EveryN(1), // durability per commit
    snapshot_seq: 0,
};
// Skip snapshots entirely. WAL replay rebuilds the graph on every restart.
// Caution: replay time grows linearly in WAL length; bound the WAL with a
// retention policy (e.g., cap at 1M entries, rotate by copy + truncate
// under a maintenance window).
```

### Server workload

Goal: durable per commit; bounded recovery time; hot backups available.

```rust
use selene_persist::{SyncPolicy, WalConfig};

let wal_config = WalConfig {
    sync_policy: SyncPolicy::EveryN(1),
    snapshot_seq: 0, // overwritten on open from the snapshot's epoch
};
// Take a snapshot hourly and after every 1M WAL entries.
// Retain the last N snapshots plus their WAL files.
// Rotate the WAL after each snapshot: open a new file seeded with
// snapshot_seq = previous_wal.last_sequence(), then remove the old WAL once
// the new snapshot is durable on disk.
```

The hot path for all three is identical — `WalWriter::append` — and differs
only in how often `fsync` runs and how often a snapshot is taken. See
[performance.md](performance.md) for the measured impact of each choice.

## Reference

- WAL writer: [`crates/selene-persist/src/writer.rs`](../crates/selene-persist/src/writer.rs)
- WAL file header: [`crates/selene-persist/src/file_header.rs`](../crates/selene-persist/src/file_header.rs)
- Snapshot writer: [`crates/selene-persist/src/snapshot_writer.rs`](../crates/selene-persist/src/snapshot_writer.rs)
- Snapshot file header: [`crates/selene-persist/src/snapshot_file_header.rs`](../crates/selene-persist/src/snapshot_file_header.rs)
- Recovery orchestrator: [`crates/selene-persist/src/recovery.rs`](../crates/selene-persist/src/recovery.rs)
- Recovery provider trait: [`crates/selene-persist/src/provider.rs`](../crates/selene-persist/src/provider.rs)
- `Change` enum: [`crates/selene-core/src/changeset.rs`](../crates/selene-core/src/changeset.rs)
- Graph-side recovery wrapper: [`crates/selene-graph/src/recover.rs`](../crates/selene-graph/src/recover.rs)
