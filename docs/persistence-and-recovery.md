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
to be re-released when graph types evolve — the producer that owns each
snapshot section family registers a `RecoveryProvider` keyed by a stable
four-byte tag, and the orchestrator routes bytes and changes to it by tag.
See [Recovery semantics for a provider](#recovery-semantics-for-a-provider).

`selene-db` is a single native engine with no extension or procedure-pack
system, so the production registry has exactly one provider: `selene-graph`
ships the `CoreProvider` (provider tag `CORE`) that reconstructs the
in-memory graph from `CORE/*` snapshot sections and post-snapshot `Change`s.
The registry's multi-provider, tag-keyed shape is what keeps `selene-persist`
graph-blind, not a plugin surface for third-party state.

Engine-owned audit events do **not** ride this provider path. They live in a
separate, dedicated append-only `audit.log` (`SLAU` magic, D24) with its own
retention policy, independent of the snapshot/WAL lineage — see
[The audit log](#the-audit-log).

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
- 2-byte flags (currently the per-section compression toggle).
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
builder.add_section(*b"CORE", *b"EDGE", core_edge_bytes)?;
let outcome = builder.finalize()?;
```

`finalize()` writes `snapshot.{sequence}.snap.tmp`, fsyncs it when
`fsync: true`, then atomically **hard-links** it to
`snapshot.{sequence}.snap` and removes the tmp. The hard-link is the
race-safe alternative to `rename` (which silently overwrites on POSIX); a
sequence collision fails fast with `Io(AlreadyExists)`. The returned
`SnapshotFinalizeOutcome` carries `snapshot_seq`, `body_hash`, and
`section_count`.

Each section's `(provider, sub)` tag pair identifies its producer. Common
tags shipped by the workspace:

| Provider | Sub     | Producer                                  |
| :------- | :------ | :---------------------------------------- |
| `CORE`   | `META`  | `selene-graph` core metadata (`CoreProvider`). |
| `CORE`   | `NODE`  | `selene-graph` node columns.              |
| `CORE`   | `EDGE`  | `selene-graph` edge columns.              |
| `CORE`   | `SCMA`  | `selene-graph` schema catalog.            |

Each producer picks a four-byte ASCII `provider` tag and a `sub` tag per
section family it owns. Tags must be globally unique within a registry —
duplicate registration fails with `PersistError::DuplicateProviderTag`. In
the single native engine the only registered producer is `selene-graph`'s
`CoreProvider`; the tag-keyed shape is the section-routing mechanism, not an
extension surface.

### When to snapshot

The engine does not take snapshots automatically; embedders drive cadence
based on workload and recovery-time budget. A reasonable starting policy:

- Take a snapshot after every `N` WAL entries (e.g., `N = 1_000_000`).
- Take a snapshot after every `T` minutes of wall-clock (e.g., hourly).
- Take a snapshot before a planned shutdown.

### Scheduling row compaction

Snapshots bound recovery time; row compaction reclaims in-memory node and edge
slots left behind by deletes. The engine exposes the maintenance operation but
deliberately does not own a timer or background runtime. Run it from the
embedder's existing maintenance cadence, preferably in a low-write window and
optionally immediately before an already-coordinated snapshot rotation.

The default recommendation requires both at least 1,024 reclaimable rows and a
reclaimable-row ratio of at least 25% of allocated rows. Reading the counters is
lock-free and does not rebuild the graph. A host that needs different size
thresholds can compare `reclaimable_rows()` and
`reclaimable_row_basis_points()` directly; an age-based policy must track the
last maintenance time in the host because graph rows do not carry deletion
timestamps. The recommendation check is advisory and separate from compaction;
serialize maintenance ticks per graph so two actors do not both perform the
same full rebuild.

```rust
use selene_graph::{CompactionReport, GraphResult, SharedGraph};

fn run_compaction_tick(
    graph: &SharedGraph,
) -> GraphResult<Option<CompactionReport>> {
    if !graph.compaction_stats().compaction_recommended() {
        return Ok(None);
    }
    graph.compact().map(Some)
}
```

`SharedGraph::compact()` is the publication boundary. It rebuilds the full live
graph while holding the writer lock, then publishes the dense snapshot in the
same ordered handoff used by commits; lock-free readers continue to observe the
previous snapshot until publication. Callers should not invoke the lower-level
`compact_core()` transform and attempt to republish its result themselves.

Compaction changes only physical row layout. It emits no `Change` and writes no
WAL entry, so the dense layout becomes durable with the next graph snapshot. A
crash before that snapshot is logically safe: recovery restores the same graph
with its prior sparse layout, and a later maintenance tick can compact it again.

GQL-driven hosts can use the equivalent native procedures: inspect
`CALL selene.compaction_stats()` and invoke maintenance-tier
`CALL selene.compact()` only when policy says to reclaim. `selene.compact()` is
a standalone maintenance statement and is not valid inside an explicit
transaction. The same scheduling and snapshot-durability rules apply.

For a scheduled snapshot rotation, use the MANIFEST-backed in-process protocol:

1. Quiesce or otherwise coordinate writes while collecting every provider
   section for one committed graph state. A single `ArcSwap` load is lock-free,
   but providers encode separate sections, so concurrent writes can otherwise
   mix generations.
2. Create an unfinalized `SnapshotBuilder` at
   `sequence = wal.last_sequence()` and add all provider sections.
3. Pass the builder to `wal.rotate_with_manifest(builder)`. The rotation flushes
   the WAL, finalizes and fsyncs the snapshot, archives the previous WAL,
   commits the MANIFEST as the epoch's linearization point, then resets the
   active `wal.log`.

## Two-step recovery

On startup, the persistence layer runs
[`recover(dir, &registry)`](../crates/selene-persist/src/recovery.rs):

```rust
use selene_persist::{ProviderRegistry, recover};

let mut registry = ProviderRegistry::new();
registry.register(core_provider.clone())?;

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

For the common case the convenience wrapper in `selene-graph` does the
registration:

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

## Recovery semantics for a provider

A `RecoveryProvider` (in the production engine, `CoreProvider`) reconstructs
its in-memory state through two surfaces:

- `read_section(sub, bytes)`: called once per snapshot section that matches
  this provider's tag, in the section table's declared order. The provider
  decodes the bytes back into its in-memory state.
- `on_change(change)`: called for every WAL `Change` past the snapshot
  sequence. The provider decides whether the change is relevant to it.

The recovery boundary has a sharp edge: **WAL replay only covers events
after the snapshot's last sequence**. Any state that is not derivable from
post-snapshot `Change`s alone must be persisted in the snapshot itself, not
left to WAL replay to rebuild — the pre-snapshot WAL entries may already
have been pruned. A section that is emitted empty on the assumption the WAL
will reconstruct it will silently lose that state on recovery.

The discipline is: if a provider holds state that affects future behavior
but is not reproducible from post-snapshot `Change`s, capture it in a
snapshot section. The `CoreProvider` follows exactly this rule — the full
node/edge/schema columns are materialized into `CORE/*` sections at
snapshot time so recovery never depends on replaying the entire WAL history.

## The audit log

Engine-owned audit events are kept in a dedicated append-only file,
`audit.log` (magic `SLAU`), that is deliberately separate from the
snapshot/WAL lineage. It is **not** a `RecoveryProvider` and does not
participate in snapshot/WAL recovery — it has its own open/append/read/prune
lifecycle and its own retention policy.

The substrate is intentionally below lifecycle semantics: each record is a
generic `kind`-tagged opaque payload plus a caller-supplied
`recorded_at_unix_nanos` wall-clock stamp (`selene-persist` does not own a
clock). The graph funnel writes WAL-first, audit-after.

```rust
use selene_persist::{AuditLog, AuditRecord, AuditRetentionPolicy};

let mut log = AuditLog::open(&dir.join("audit.log"))?;
log.append(&AuditRecord {
    recorded_at_unix_nanos: now_unix_nanos,
    kind: 1,
    payload: payload_bytes,
})?;

// Independent retention: keep the newest N events and/or drop events older
// than `max_age`. Both constraints default to unbounded and are conjunctive.
let policy = AuditRetentionPolicy {
    keep_n_events: Some(100_000),
    max_age: None,
};
let _outcome = log.prune(&policy, now_unix_nanos)?;
```

`open` performs a torn-tail-truncating scan (a partial trailing record is
dropped, not an error), mirroring the WAL's on-open posture. Recovery
reattaches the audit log purely by file presence. Retention here is
independent of the snapshot/WAL `RetentionPolicy` (see [Backups](#backups)),
so trimming audit history never affects graph recovery and vice versa.

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
evolve them. The first-party `CORE` sections do this through subsection
version bytes inside their rkyv-archived bodies — for example the `CORE/GTYP`
section carries its own `GTYP_VERSION` independent of the `SLSN` container
version.

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
// Caution: replay time grows linearly in WAL length. If that becomes too slow,
// adopt coordinated snapshots and WalWriter::rotate_with_manifest rotations;
// prune superseded snapshots and WAL archives separately with RetentionPolicy.
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
// Rotate each populated SnapshotBuilder with wal.rotate_with_manifest(builder).
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
- Graph compaction policy and transform: [`crates/selene-graph/src/compaction.rs`](../crates/selene-graph/src/compaction.rs)
- Ordered live compaction publication: [`crates/selene-graph/src/shared.rs`](../crates/selene-graph/src/shared.rs)
