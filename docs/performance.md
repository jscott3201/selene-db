# Performance

This document is for engineers tuning `selene-db`, evaluating it against a
workload, or comparing it against other graph engines. It pulls the
headline numbers from [BENCHMARKS.md](../BENCHMARKS.md), describes the
measurement methodology, and walks through the tuning knobs that materially
affect throughput and latency.

The numbers below are measured locally on a single machine; `selene-db` does
not run benchmarks in CI. The canonical record is `BENCHMARKS.md`, which is
refreshed on a manual cadence and committed to the repository.

## Headline numbers

The most recent measurements (Apple M5, 10 cores, 16 GiB, rustc 1.97.1):

| Workload                                  | Result          | Notes                                              |
| :---------------------------------------- | :-------------- | :------------------------------------------------- |
| Node fetch (`graph_node_fetch`)           | **7.42 ns**     | Near-flat across 10k / 50k / 100k (6.18 → 7.42 ns); columnar fetch. |
| Typed index point lookup                  | **11.92 ns**    | Flat across scales; tri-state `Cow<RoaringBitmap>` |
| Semantic analyze (m5c corpus)             | **21.98 µs**    | Strict ISO GQL analysis on the representative corpus (`gql_analyze_corpus/m5c`). |
| Betweenness centrality (100k nodes, parallel) | **101.7 ms**    | 2.6× speedup over sequential at 100k.              |
| WAL append (single, batched x1000)        | **10.28 ms / 100k entries** | Group-commit dominates; ~59× faster than per-entry. |
| Full recovery (snapshot + 100k WAL)       | **16.31 ms**    | Snapshot reconciliation + WAL 3.0 replay.          |

These are wall-clock medians from criterion 0.8. They do **not** include
client-side serialization, network round-trips, or any layer that
`selene-db` does not own. Embedders that wrap the engine in a server should
measure end-to-end through that server.

## How to read the benchmarks

`selene-db` uses two measurement layers:

- **criterion 0.8** for wall-clock medians. This is the primary signal.
  Reported numbers in `BENCHMARKS.md` are criterion medians.
- **iai-callgrind** for instruction-count baselines. These are pinned tokens
  in `BENCHMARKS.md` and populated during release-prep cycles; they require
  `valgrind` and are Linux-only in practice.

Three conventions are load-bearing:

1. **Sequential execution.** Every bench binary runs in isolation. Criterion
   dispatches its measurements concurrently within a binary, which is fine,
   but `cargo bench --workspace` dispatches multiple bench binaries
   concurrently, which corrupts every measurement. The repository's runner
   script enforces sequencing with a `pgrep` guard.
2. **Bench-binary allocator pinning.** Every bench binary in the workspace
   pins `mimalloc` as the global allocator via a tiny `mimalloc` dev-dep.
   This isolates the measurements from the embedder's allocator choice and
   makes the numbers reproducible across hosts. Library crates do **not**
   depend on `mimalloc`; embedders pick their own allocator.
3. **Deterministic fixtures.** Bench fixtures are deterministic
   (`BenchFixture::build(N)` and friends from `selene-testing`). The same N
   produces the same graph and the same query mix.

Benchmarks are intentionally **local-only**. There is no GitHub Actions
benchmark job, no gh-pages trend dashboard, no per-PR regression gate. The
reasoning: CI runners are noisy, varied, and short-lived; trend tracking
that conflates GitHub-runner noise with code changes is worse than no trend
tracking. The committed `BENCHMARKS.md` is the canonical history; updates
are intentional events tied to specific hardware footprints.

## Running benchmarks locally

The sanctioned entry point is the runner script:

```bash
scripts/run-benches.sh --profile full --layer criterion
```

Profiles select workload envelopes from
`selene-testing::bench_profiles::BenchProfile`:

| Profile  | Scale envelope                              |
| :------- | :------------------------------------------ |
| `quick`  | Short local smoke runs; sub-minute total.   |
| `full`   | Publish-quality `BENCHMARKS.md` refresh.    |
| `stress` | Opt-in larger envelopes for stress.         |

Layers select the measurement backend:

| Layer       | What runs                                                                     |
| :---------- | :---------------------------------------------------------------------------- |
| `criterion` | criterion 0.8 wall-clock medians.                                             |
| `iai`       | iai-callgrind instruction-count baselines; requires `valgrind`.               |
| `both`      | criterion plus iai where `valgrind` is available.                             |

A typical refresh pass before updating `BENCHMARKS.md`:

```bash
git checkout main && git pull --ff-only
scripts/run-benches.sh --profile full --layer criterion
# Then update BENCHMARKS.md with the new medians and hardware footprint.
```

### Why not `cargo bench --workspace`?

Two reasons. First, Cargo dispatches each member's bench binaries
concurrently; with two or more crates exposing bench targets the
measurements all corrupt each other. Second, the runner script captures
hardware metadata (CPU brand string, core count, memory, rustc version,
commit) that goes into the `BENCHMARKS.md` header — running `cargo bench`
directly skips that metadata capture.

If you need to run a single bench, the runner takes a `--filter` argument
that passes through to criterion:

```bash
scripts/run-benches.sh --profile quick --layer criterion --filter graph_node_fetch
```

## Tuning knobs

The major levers that affect throughput and latency, in roughly the order
they come up in practice:

### Fsync policy

**Which knob applies depends on which path you are on, and the two are easy to
confuse.**

*Live `SharedGraph`* — the committer owns fsync, and `with_wal` **discards** the
`SyncPolicy` you pass, forcing `OnFlushOnly` so the committer's group flush is
the single durability barrier. The knob that matters here is
`CommitBatching`: `Off` is one fsync per commit; `On { max_commits, max_bytes }`
coalesces a contiguous run of commits into one fsync, trading latency for
throughput. Setting `SyncPolicy` on this path has no effect.

*Offline `WalWriter` tooling* — `SyncPolicy` means what it says. `EveryN(1)`
fsyncs after every appended entry, `EveryN(N)` amortizes across N, and
`OnFlushOnly` defers to an explicit `flush()`.

The numbers below are the raw-writer path, which is what the WAL benchmark
measures; treat them as the shape of the fsync trade-off rather than as live
commit latency.

Measured on the raw writer at 10k changes, no compression (macOS, 10-core M5,
2026-08-16). Every row is one `fsync` cadence over the same total work:

```bash
scripts/run-benches.sh --profile full --bench wal
```

| Entry shape             | Policy          | Time     | Throughput        |
| :---------------------- | :-------------- | :------- | :---------------- |
| 10k × 1 change          | `EveryN(1)`     | 35.97 s  | ~280 changes/s    |
| 10k × 1 change          | `EveryN(10)`    | 3.668 s  | ~2.7k changes/s   |
| 10k × 1 change          | `EveryN(100)`   | 475.8 ms | ~21k changes/s    |
| 10k × 1 change          | `EveryN(1000)`  | 60.61 ms | ~165k changes/s   |
| 10k × 1 change          | `OnFlushOnly`   | 17.18 ms | ~582k changes/s   |
| 10 × 1000 changes       | `EveryN(1000)`  | 6.295 ms | ~1.6M changes/s   |

The first five rows are `persist_wal_sync_sweep`; the last is
`persist_wal_append_batch_1000`. **`EveryN(1)` is nearly four orders of
magnitude off the bottom row.** An earlier revision of this table labelled its
single row `EveryN(1)` while quoting a benchmark that runs `EveryN(1000)`;
measured at 10k those two policies differ by ~590×, so that row understated
per-entry fsync by about that much. Size durability budgets from this table.

Two separate wins are visible and it is worth keeping them apart. Rows 1-5 are
**fsync amortization**: the cost is syscall latency, not selene-db code. The
last row adds **entry batching** — one `commit` per multi-`Change` transaction
rather than one per `Change` — which is a further ~2.7× over `OnFlushOnly` at
equal fsync count, from per-entry framing and checksum work avoided.

See [persistence-and-recovery.md](persistence-and-recovery.md) for the full
`SyncPolicy` semantics and the append -> flush -> publish -> acknowledge
commit ordering.

### Parallelization gating

Graph algorithm parallelization is controlled by the
`Parallelism` enum in
[`selene-algorithms`](../crates/selene-algorithms/src/parallel.rs):

```rust
pub enum Parallelism {
    Sequential,
    Auto, // default
    Threads(NonZeroUsize),
}
```

`Auto` uses the current rayon pool size (or the global pool size if not
already inside a rayon scope). `Threads(N)` builds an explicit pool. The
algorithm authors decide per-algorithm whether `Auto` actually parallelizes
or falls back to sequential — see the algorithm-by-algorithm notes in
[graph-algorithms.md](graph-algorithms.md).

A subtle but important fact: parallelization is not always faster. The
`pagerank` benchmarks show `Auto` is **slower** than `Sequential` on the
~3 edges/node bench graph at every measured scale — coordination cost
outweighs the per-iteration work. The API exposes both modes deliberately;
parallelization is a tuning knob, not a default.

## Read performance

The two read paths that dominate query latency are node fetch and typed
index point lookup. Re-measured 2026-08-16 with
`scripts/run-benches.sh --profile full --bench single_graph`:

| Path                       | 10k     | 50k     | 100k    | Shape                                            |
| :------------------------- | :------ | :------ | :------ | :----------------------------------------------- |
| `graph_node_fetch`         | 6.18 ns | 7.17 ns | 7.42 ns | O(1); columnar fetch by `NodeId`.                |
| `graph_typed_index_point`  | 11.89 ns | 11.95 ns | 11.92 ns | Flat-curve via tri-state `Cow<'_, RoaringBitmap>` lookup; an `FxHashMap` keyed by typed-value handles. |
| `graph_label_index_lookup` | 11.24 ns | 10.90 ns | 11.06 ns | `DbString`-keyed hash lookup against the label index. Regressed ~37–44% vs the 2026-06-01 baseline; see the `BENCHMARKS.md` §2 note. |

These are honest measured numbers — the node fetch is two cache-line probes
plus a `RoaringBitmap` membership check on the live bitmap; the typed
index lookup is an `FxHashMap` lookup plus the same membership check. They
are the floor `selene-db` operates at.

Both index lookups are flat across scales. Node fetch is *near*-flat: it rises
about 20% from 10k to 100k (6.18 → 7.42 ns), which is cache behaviour over a
larger row store rather than an algorithmic term — the lookup itself stays
O(1).

Range scans and composite lookups scale sub-linearly (see the
`graph_typed_index_range` and `graph_composite_index_proxy` rows in
`BENCHMARKS.md`).

## Write performance

The write path is dominated by the WAL append and the commit-time index
maintenance. Every row below is measured at the **100k-node fixture**, on the
hardware described in `BENCHMARKS.md`, re-measured 2026-08-16 with
`scripts/run-benches.sh --profile full --bench bulk_mutation` and
`--bench wal`. `BENCHMARKS.md` is the baseline of record; this table is a
summary of it and must not be edited independently.

| Workload                                  | Time @100k | Notes                                              |
| :---------------------------------------- | :--------- | :------------------------------------------------- |
| `graph_edge_create_cascade`               | 1.417 ms   | Mutation + commit body; teardown excluded.          |
| `graph_mutation_commit_batch` (batch=10)  | 127.8 µs   | Three-clone cascade replaced with one COW clone at first mutation. |
| `graph_mutation_commit_batch` (batch=100) | 187.2 µs   | Batching wins at higher cardinality.                |
| `graph_mutation_commit_batch` (batch=1000)| 521.9 µs   | Per-Change amortization plateaus around batch=1000. |
| `persist_wal_append_batch_1000`           | 10.28 ms   | ~59× faster than per-entry at 100k (604.7 ms).      |

The commit path is **flat-curve**: in-place mutation against the
`Arc<SeleneGraph>` write lock with one COW clone at first mutation,
followed by explicit `pre_txn` rollback semantics on error. The
three-clone cascade that the early implementation carried was retired —
the current shape replaces it with one COW clone at first mutation and
keeps subsequent mutations in-place.

Embedders that batch their mutations into multi-`Change` transactions
benefit twice: from amortized commit-side bookkeeping and from
group-commit WAL fsync.

## Graph algorithm performance

The graph algorithm library parallelizes through `rayon`. Rows below are the
`BENCHMARKS.md` §6a values verbatim; §6a is the baseline of record and carries
the **2026-06-01** file stamp, so unlike the write-path table above these were
not re-measured on 2026-08-16. Scale is the node count except for APSP, where
it is the **source count** (~3 edges/node for pagerank/betweenness/apsp; ~6
edges/node for triangle_count/louvain).

| Algorithm        | Scale | Sequential | Parallel (`Auto`) | Speedup |
| :--------------- | :---- | :--------- | :---------------- | :------ |
| PageRank         | 100k  | 1.058 ms   | 1.050 ms          | 1.01×   |
| Betweenness      | 100k  | 266.1 ms   | 101.7 ms          | 2.6×    |
| Triangle count   | 100k  | 6.345 ms   | 4.888 ms          | 1.30×   |
| APSP (200 src)   | 200   | 621.8 µs   | 306.5 µs          | 2.03×   |
| APSP (1k src)    | 1000  | 17.17 ms   | 5.576 ms          | 3.1×    |
| Louvain          | 100k  | 18.57 ms   | n/a               | sequential-only |

Notable points:

- **`Auto` no longer picks the parallel PageRank kernel at all.** Rayon
  overhead lost at every measured scale on the sparse bench graph
  (~3 edges/node) — the per-iteration work is small (3·N FP multiplications
  + accumulator) — so `Auto` now routes to the sequential kernel and the two
  columns are the same measurement to within noise. An explicit `Threads(n)`
  still opts into the parallel kernel for caller-forced experiments.
- **Betweenness parallel scales strongly**: per-source SSSP is independent
  work, so the parallelism budget is well spent.
- **APSP parallel scales with source count**, reaching 3.1× at 1k sources on
  10 cores. It is not the workspace's single strongest parallel win —
  betweenness reaches 3.3× `Auto` at 10k (see §6a).
- **Louvain is sequential-only** today — the modularity-optimization
  iteration carries cross-vertex dependencies that the current
  implementation does not parallelize safely.

## What gets worse at scale

`selene-db` is engineered to be embedded as a library in-process, and the
operating envelope is shaped by that. Some honest limits:

- **Memory footprint per node**: a node carries its `NodeId`, its label
  set, its property map, and entries in every secondary index it
  participates in. Property values that are large strings or large
  `Value::Binary` instances dominate the footprint at scale. Plan storage
  budget accordingly.
- **APSP is `O(N · (V + E) · log V)`**. For 100k nodes the workspace
  measures the bench at 200/500/1000 sources, not all-pairs over 100k.
  All-pairs at 100k is not a practical workload for any single-machine
  graph engine and `selene-db` is no exception.
- **Betweenness sampling vs exact**: exact betweenness is `O(N · (V + E))`
  per source, all sources. Sampled betweenness uses endpoint-aware spacing
  (the bench-corpus configuration), trading exactness for a sub-second
  budget at 100k. The implementation does not silently sample — the caller
  asks for exact or sampled explicitly.
- **WAL replay is linear in WAL length**. Snapshot rotation is the
  bounded-recovery mechanism; an unbounded WAL is unbounded recovery time.
- **Strict-serializable transactions take a single write lock**. Lock-free
  reads scale across cores, but write throughput is capped by the
  single-writer envelope. Long-running write transactions block all other
  writers until commit.

## Comparing against other engines

`selene-db`'s posture is "embeddable library with strict ISO/IEC 39075:2024
GQL semantics; in-process; no transport". Other graph engines optimize
different axes — distributed deployment, looser query-language conformance,
external storage, multi-graph hosting, mixed OLAP/OLTP workloads. Direct
numeric comparison between engines with different deployment models, query
languages, and storage shapes is not informative.

The repository does not publish "we are faster than X" claims. The honest
posture is to define the workload — graph shape, query mix, persistence
configuration, hardware — and let users run their own comparison.

A reproducible workload definition is straightforward:

```text
Graph:        100k nodes, 300k edges, label-uniform.
Indexes:      One typed index on `:Person.email`.
Persistence:  SharedGraph with_wal, CommitBatching::Off, snapshot at start.
Query mix:    80% MATCH (n:Person {email: $e}) RETURN n
              20% MATCH (n:Person)-[:KNOWS]->(m) RETURN m LIMIT 10
Hardware:     10-core M5, 16 GiB, macOS, rustc 1.95.0.
```

`selene-db` runs this workload through the embedded library API; any
engine the embedder is evaluating runs the equivalent through its own
deployment. The criterion fixtures in
[`crates/selene-testing`](../crates/selene-testing) and
[`crates/selene-graph/benches`](../crates/selene-graph/benches) are the
starting point for adapting workloads.

## Reference

- Canonical benchmark record: [BENCHMARKS.md](../BENCHMARKS.md)
- Bench runner: [`scripts/run-benches.sh`](../scripts/run-benches.sh)
- Bench profile envelopes: `crates/selene-testing/src/bench_profiles.rs`
- Parallelism policy: [`crates/selene-algorithms/src/parallel.rs`](../crates/selene-algorithms/src/parallel.rs)
- WAL writer: [`crates/selene-persist/src/writer.rs`](../crates/selene-persist/src/writer.rs)
