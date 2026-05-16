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

The most recent measurements (Apple M5, 10 cores, 16 GiB, rustc 1.95.0):

| Workload                                  | Result          | Notes                                              |
| :---------------------------------------- | :-------------- | :------------------------------------------------- |
| Node fetch (`graph_node_fetch`)           | **2.10 ns**     | Flat across 10k / 50k / 100k; columnar fetch.      |
| Typed index point lookup                  | **4.53 ns**     | Flat across scales; tri-state `Cow<RoaringBitmap>` |
| Semantic analyze (m5c corpus)             | **5.32 µs**     | Strict ISO GQL analysis on the representative corpus. |
| Betweenness centrality (100k nodes, parallel) | **110.2 ms**    | 2.40× speedup over sequential at 100k.             |
| IVF search (k=10, dim=256, 256 vectors)   | **2.88 µs**     | Trained IVF; ~6× faster than HNSW at this corpus.  |
| HNSW build (n=1000, dim=16, M=8, efC=64)  | **53.25 ms**    | Direct construction with the cold-start tuning.    |
| WAL append (single, batched x1000)        | **10.95 ms / 100k entries** | Group-commit dominates; 54× faster than per-entry. |
| Full recovery (snapshot + 100k WAL)       | **24.75 ms**    | Snapshot reconciliation + WAL v2 replay.           |

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
   produces the same graph, the same vector corpus, the same query mix.

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
scripts/run-benches.sh --profile quick --layer criterion --filter vector_hnsw_build
```

## Tuning knobs

The major levers that affect throughput and latency, in roughly the order
they come up in practice:

### WAL `SyncPolicy`

Durability versus write throughput. The default `EveryN(1)` issues an
`fsync` after every appended entry — durable per commit. `EveryN(N)` with
`N > 1` opts into group commit, amortizing fsync across batches at the cost
of up to `N-1` entries of recent durability. `OnFlushOnly` suppresses fsync
entirely except on explicit `flush()`; do not use this in production.

Measured impact at 100k WAL entries (single appends, no compression):

| Policy           | Time      | Throughput            |
| :--------------- | :-------- | :-------------------- |
| `EveryN(1)`      | 588.9 ms  | ~170k entries/s       |
| `OnFlushOnly`    | 108.6 ms  | ~920k entries/s       |
| Batched (1000)   | 10.95 ms  | ~9.1M entries/s       |

Batching at the embedder layer (one `commit` per multi-`Change` transaction
rather than one `commit` per `Change`) is the biggest single win.

See [persistence-and-recovery.md](persistence-and-recovery.md) for the full
`SyncPolicy` semantics.

### HNSW configuration

The HNSW provider exposes the standard three knobs through its construction
config: `M` (max edges per node per layer), `ef_construction` (beam width
during build), and `ef_search` (beam width during query).

| Knob              | Direction                  | Default tuning                                |
| :---------------- | :------------------------- | :-------------------------------------------- |
| `M`               | Higher = better recall, larger memory footprint and slower build. | 8–16 typical; build cost is `O(N · M²)`. |
| `ef_construction` | Higher = better recall, slower build. | 64–200 typical.                              |
| `ef_search`       | Higher = better recall, slower query. | Workload-dependent; tune against a target recall. |

The `vector_hnsw_build` benchmark in `BENCHMARKS.md` uses `M=8`,
`ef_construction=64` at dim 16 — small enough to fit comfortably in cache
and a tractable baseline. Production workloads at dim 384–1536 will be
substantially slower per insert; the `M²` factor in build cost is the
dominant scaling term.

### IVF configuration

The IVF provider exposes coarse-quantizer training (`n_centroids`) and
search-time `n_probe`. The trade-off:

| Knob          | Direction                                                       |
| :------------ | :-------------------------------------------------------------- |
| `n_centroids` | Higher = smaller per-probe scan, more memory for centroids.     |
| `n_probe`     | Higher = better recall, linear cost increase.                   |

The `vector_ivfpq_recall_at_10` row in `BENCHMARKS.md` shows the linear
scaling:

| `n_probe` | Search latency |
| :-------- | :------------- |
| 1         | 699.7 ns       |
| 4         | 2.68 µs        |
| 8         | 4.97 µs        |

Sub-µs cold-cache lookups at `n_probe=1`, linear up from there.

### Quantization

Three quantizers ship in `selene-vector`:

| Quantizer | Bytes per vector (dim D)        | Recall posture (with rescore tier) |
| :-------- | :------------------------------ | :--------------------------------- |
| `f32`     | `4 · D`                         | Baseline.                          |
| `SQ8`     | `D + small overhead`            | Essentially free; 0.994 recall@10 with rescore, ~4× memory reduction. |
| `PQ`      | `M_pq · log2(K) / 8` (small)    | Collapses to ~0.5 alone; 0.984 with rescore tier. |
| `OPQ`     | Same footprint as PQ.           | Same recall posture as PQ + rescore; ~48× slower on insert due to rotation. |

The pattern that wins for vector search is **quantize for memory, pair with
a rescore tier for recall**. SQ8 by itself is the fastest cheap option. PQ
and OPQ are only worth it when memory is the tight constraint and a rescore
tier is configured to recover recall.

OPQ rotation on insert is expensive: the `composition_replay` benchmarks
measure ~48× slower inserts than plain PQ. Favor `plain_pq` unless the
workload is read-dominant and OPQ's polysemous code-tying is needed.

### Procedure-pack registry: lazy vs eager activation

The procedure-pack registry (`selene-pack::ProcedureRegistry`) activates
packs lazily by default — the first `CALL` against a pack triggers manifest
validation and typestate transition. Eager activation (activating every
pack at startup) trades startup latency for steady-state predictability.

For workloads with a small fixed set of `CALL`s on the hot path, eager
activation removes the once-per-pack cold path. For workloads that touch
many packs sparsely, lazy activation keeps startup fast.

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
index point lookup. Both are measured flat across scales:

| Path                       | 10k     | 50k     | 100k    | Shape                                            |
| :------------------------- | :------ | :------ | :------ | :----------------------------------------------- |
| `graph_node_fetch`         | 2.10 ns | 2.11 ns | 2.09 ns | O(1); columnar fetch by `NodeId`.                |
| `graph_typed_index_point`  | 4.53 ns | 4.44 ns | 4.67 ns | Flat-curve via tri-state `Cow<'_, RoaringBitmap>` lookup; an `FxHashMap` keyed by typed-value handles. |
| `graph_label_index_lookup` | 5.20 ns | 4.24 ns | 4.30 ns | `IStr`-keyed hash lookup against the label index. |

These are honest measured numbers — the node fetch is two cache-line probes
plus a `RoaringBitmap` membership check on the live bitmap; the typed
index lookup is an `FxHashMap` lookup plus the same membership check. They
are the floor `selene-db` operates at, and they do not regress with graph
size.

Range scans and composite lookups scale sub-linearly (see the
`graph_typed_index_range` and `graph_composite_index_proxy` rows in
`BENCHMARKS.md`).

## Write performance

The write path is dominated by the WAL append and the commit-time index
maintenance. Measurements (100k-node fixture):

| Workload                                  | Time      | Notes                                              |
| :---------------------------------------- | :-------- | :------------------------------------------------- |
| `graph_edge_create_cascade`               | 343.5 µs  | Mutation + commit body; teardown excluded.          |
| `graph_mutation_commit_batch` (batch=10)  | 360.8 µs  | Three-clone cascade replaced with one COW clone at first mutation. |
| `graph_mutation_commit_batch` (batch=100) | 434.0 µs  | Batching wins at higher cardinality.                |
| `graph_mutation_commit_batch` (batch=1000)| 761.9 µs  | Per-Change amortization plateaus around batch=1000. |
| `persist_wal_append_batch_1000`           | 10.95 ms / 100k | 54× faster than per-entry at 100k.            |

The commit path is **flat-curve**: in-place mutation against the
`Arc<SeleneGraph>` write lock with one COW clone at first mutation,
followed by explicit `pre_txn` rollback semantics on error. The
three-clone cascade that the early implementation carried was retired —
the current shape replaces it with one COW clone at first mutation and
keeps subsequent mutations in-place.

Embedders that batch their mutations into multi-`Change` transactions
benefit twice: from amortized commit-side bookkeeping and from
group-commit WAL fsync.

## HNSW + IVF performance

Build, search, and mutation throughput for the vector index extension.
Search is the primary cost on most workloads; build is amortized; mutation
is rare in batch-ingest patterns but hot in streaming patterns.

Build (HNSW, direct construction, dim=16, M=8, ef_construction=64,
deterministic seed):

| n    | Time      |
| :--- | :-------- |
| 100  | 2.79 ms   |
| 1000 | 53.25 ms  |
| 5000 | 340.09 ms |

Cost scales roughly as `O(n · log n · M²)` — the 5× increase from 1k to 5k
nodes costs ~6.4× wall-clock.

Search (HNSW, k=10, recall@10):

| Variant                | k=10       | k=25       | k=50      | k=100     |
| :--------------------- | :--------- | :--------- | :-------- | :-------- |
| `f32` baseline         | 705.6 µs / 0.881 | 1.24 ms / 0.991 | 2.18 ms / 1.000 | 4.60 ms / 1.000 |
| `SQ8` + rescore        | 749.3 µs / 0.884 | 1.28 ms / 0.991 | 2.23 ms / 1.000 | 4.81 ms / 1.000 |
| `PQ` + rescore         | 746.0 µs / 0.503 | 1.26 ms / 0.778 | 2.30 ms / 0.931 | 4.92 ms / 0.984 |
| `OPQ` + rescore        | 733.3 µs / 0.500 | 1.26 ms / 0.781 | 2.31 ms / 0.925 | 4.95 ms / 0.994 |

(latency / recall@10.)

Bulk mutation (HNSW vs IVF, 100-vector batch, dim=256):

| Provider | Bulk-upsert time | Per-vector time |
| :------- | :--------------- | :-------------- |
| HNSW     | 4.22 ms          | 42 µs           |
| IVF      | 57.37 µs         | 0.57 µs         |

IVF bulk mutation is roughly 70× faster than HNSW at this batch size
because IVF appends to posting lists rather than inserting into a
hierarchical graph.

## Graph algorithm performance

The graph algorithm library parallelizes through `rayon`. Numbers from the
100k-node fixture (~3 edges/node for pagerank/betweenness/apsp; ~6
edges/node for triangle_count/louvain):

| Algorithm        | Scale | Sequential | Parallel (`Auto`) | Speedup |
| :--------------- | :---- | :--------- | :---------------- | :------ |
| PageRank         | 100k  | 2.94 ms    | 4.49 ms           | 0.65×   |
| Betweenness      | 100k  | 264.7 ms   | 110.2 ms          | 2.40×   |
| Triangle count   | 100k  | 10.33 ms   | 8.86 ms           | 1.17×   |
| APSP (200 src)   | 200   | 1.52 ms    | 466.1 µs          | 3.27×   |
| APSP (1k src)    | 1000  | 34.54 ms   | 8.46 ms           | 4.08×   |
| Louvain          | 100k  | 55.43 ms   | n/a               | sequential-only |

Notable points:

- **PageRank parallel is slower** on the sparse bench graph (~3 edges/node)
  at every measured scale. The per-iteration work is small (3·N FP
  multiplications + accumulator); rayon's thread-coordination cost
  outweighs the parallelism gain. On denser graphs (≥ 10 edges/node)
  parallel pagerank pays off; the API exposes both modes deliberately.
- **Betweenness parallel scales strongly**: per-source SSSP is independent
  work, so the parallelism budget is well spent.
- **APSP parallel scales near-linearly** up to the core count; the 4.08×
  speedup at 1k sources on 10 cores is the strongest parallel win in the
  workspace.
- **Louvain is sequential-only** in v1.0 — the modularity-optimization
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
- **HNSW build is `O(N · log N · M²)`**. At dim 768+ with `M=16`, build
  time for millions of vectors is significant. IVF training is faster but
  carries a separate trade-off (no incremental refinement).
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
Persistence:  WAL SyncPolicy::EveryN(1), snapshot at start.
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
- Vector index: [`crates/selene-vector`](../crates/selene-vector)
