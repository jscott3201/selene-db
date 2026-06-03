# selene-db benchmarks

_Last measured: 2026-06-01 on Apple M5 (10-core / 16 GiB / macOS 26.5 build 25F71 / rustc 1.95.0 / commit `3a864ac`)._

This file is the **north-star performance baseline** for selene-db: the medians
below are the reference point for performance-uplift work and the tripwire for
regressions. Every benchmark bin registered in `scripts/run-benches.sh` is
documented here — that parity is CI-enforced (fast-gate
`.github/scripts/check-benchmarks-doc.sh` + release-gate
`crates/selene-testing/tests/benchmarks_md_pin.rs`), so a bench can never ship
undocumented (the historical `expression_eval` orphan).

The suite is **criterion-only** (wall-clock medians on the dev box). There is no
iai-callgrind instruction-count layer — it needs valgrind, which never runs on
the macOS dev machine, so it was dropped rather than left as a perpetually-TBD
placeholder.

## Running benchmarks

`scripts/run-benches.sh` is the sanctioned entry point. Direct `cargo bench
--workspace` is **forbidden** (Cargo may dispatch bench binaries concurrently,
which corrupts wall-clock medians); the runner executes strictly one binary at a
time, guarded by a `pgrep` check and a serial run loop.

The runner is **flexibly scoped** so you never have to fire up a whole run to
check one thing:

```bash
run-benches.sh --list                          # enumerate registered benches + smoke subset
run-benches.sh --smoke                          # curated <~60s tripwire subset (profile quick)
run-benches.sh                                  # FULL run, every bench (the north-star sweep)
run-benches.sh --bench wal                      # one bench bin (scoped compile + run)
run-benches.sh --crate selene-graph             # every bench in one crate
run-benches.sh --bench wal --filter body_size   # one criterion group within a bin
run-benches.sh --bench graph_hub_delete --sample-size 50 --measurement-time 5   # A/B fidelity knobs
run-benches.sh --crate selene-algorithms --dry-run   # preview resolved invocations, run nothing
```

### Profiles

The workload envelope is selected by `SELENE_BENCH_PROFILE` (set via `--profile`;
see `crates/selene-testing/src/bench_profiles.rs`):

| Profile | Scales | Sample / measurement | Use |
|---|---|---|---|
| `quick` | one 1k scale | 10 / 500 ms | fast spot-check / smoke |
| `full` (default) | 10k / 50k / 100k | 30 / 1500 ms | **publish-quality / this doc** |
| `stress` | adds 250k | 30 / 1500 ms | opt-in larger envelope |

A few benches sweep an independent axis instead of the node-scale envelope (hub
degree, WAL entry-body packing, WAL sync policy, writer fan-in, correlated-row
count); those are profile-trimmed too so `quick` stays fast.

## Tracking regressions

The full sweep saves a named criterion baseline so later runs report a
percentage delta instead of a bare median:

```bash
run-benches.sh --profile full --save-baseline northstar     # record this doc's baseline
# … later, after an optimization or to check for a regression …
run-benches.sh --bench graph_hub_delete --baseline northstar # %-change vs northstar
```

`--save-baseline` and `--baseline` are mutually exclusive (one records, one
compares). criterion stores baselines under `target/criterion/` (gitignored, so
local to your machine); the committed baseline of record is the number tables in
this file. Refresh both together (see [Update protocol](#update-protocol)) on a
quiet machine — background load pollutes the medians.

## Hardware footprint

| Field | Value | Source |
|---|---|---|
| CPU | Apple M5 | `sysctl -n machdep.cpu.brand_string` |
| Cores | 10 physical / 10 logical | `sysctl -n hw.physicalcpu hw.logicalcpu` |
| Memory | 16.0 GiB | `sysctl -n hw.memsize` |
| OS | macOS 26.5 (build 25F71) | `sw_vers` |
| rustc | 1.95.0 (59807616e 2026-04-14) | `rustc --version` |
| Commit | `3a864ac` | `git rev-parse --short HEAD` |

All bench binaries use `mimalloc` as the global allocator; the library crates
are allocator-agnostic.

## §1 selene-core

Bench bin: `value_clone`. Measures `Value` / `PropertyMap` clone cost, which is
dominated by `size_of::<Value>()` — every clone memcpys the whole enum regardless
of the active variant. A compile-time `size_of::<Value>() <= 32` ceiling in
`value.rs` is the zero-cost re-bloat tripwire; the bench prints the live size to
stderr. **Measured `size_of::<Value>() = 32 bytes`** at this commit — CORE-06
boxed the four oversized variants (`Path` 120 B — the real former ceiling, not
the time variants — plus `Duration`/`ZonedDateTime`/`ZonedTime`), down from 128 B.
The same bin also covers the wide-map construction path so `from_pairs` stays
linearithmic rather than repeated-insert quadratic for schema- or record-shaped
maps with many properties. The `core_vector_value/*` rows are the first native
vector baselines: validation/construction, `Arc<[f32]>` clone cost, and postcard
round-trip cost at common embedding dimensions. The `core_vector_distance/*`
and `core_vector_exact_top_k/*` rows are exact-search oracle baselines for the
future ANN layer; current kernels are safe scalar `f64` accumulators, so these
rows are the SIMD/Rayon improvement tripwire.

| Bench | Median | Notes |
|---|---:|---|
| `core_value_clone/vec_mixed_1024` | 4.62 µs | Clone a 1024-element mixed-variant `Vec<Value>`. **−25%** vs the 128 B layout (was 6.10 µs). |
| `core_value_clone/property_map_5` | 53.8 ns | Clone a 5-key `PropertyMap` (Int/Float/String/Duration/ZonedDateTime). **−29%** (was 74.6 ns) — the *worst* case: 2 of 5 keys are now boxed (alloc-on-clone), so non-temporal maps gain more. |
| `core_value_clone/property_map_from_pairs_256_reverse` | 3.34 µs (quick) | Build a 256-property map from reverse-sorted pairs. PR-local quick A/B: 32.2 µs → 3.34 µs after the sort+dedup constructor rewrite. |
| `core_vector_value/construct_validate/128/768/1536` | 55.4 ns / 276 ns / 528 ns (quick) | Validate finite, non-empty `f32` vectors while constructing `VectorValue`; roughly linear in dimension. |
| `core_vector_value/clone_arc/128/768/1536` | 3.12 ns / 3.12 ns / 3.13 ns (quick) | Clone `VectorValue` shared component storage; intentionally dimension-independent. |
| `core_vector_value/postcard_roundtrip/128/768/1536` | 240 ns / 1.04 µs / 2.07 µs (quick) | Serialize and deserialize `Value::Vector`, including deserialize-time invariant checks. |
| `core_vector_distance/squared_euclidean/128/768/1536` | 39.2 ns / 333 ns / 713 ns (quick) | Exact lower-is-better L2-squared metric, scalar `f64` accumulation. |
| `core_vector_distance/cosine/128/768/1536` | 88.4 ns / 918 ns / 2.02 µs (quick) | Exact cosine distance with zero-norm checks and clamped similarity. |
| `core_vector_distance/negative_inner_product/128/768/1536` | 33.8 ns / 326 ns / 706 ns (quick) | Max-inner-product adapter (`-dot`) with lower-is-better ordering. |
| `core_vector_exact_top_k/squared_euclidean_2048x128_k10` | 84.7 µs (quick) | Exhaustive exact-search oracle over 2,048 candidates using a bounded max-heap (`O(n log k)`). |

## §2 selene-graph — read hot paths

Bench bins: `single_graph`, `vector_index_rebuild`, `bulk_mutation`,
`concurrent_read`, `bfs`. The medians below predate CORE-06 (measured at the
128 B `Value` layout); now that `Value` is 32 B, the `PropertyMap`-clone-heavy
rows (`graph_edge_create_cascade`, `graph_mutation_commit_batch`) will tighten
at the next full re-baseline. `graph_node_fetch` returns a column ref (no
`Value` clone) and is unaffected. `graph_exact_vector_scan/*` is the native
graph-level exact-vector oracle: label-filtered row scan plus scalar metric
kernel, returning stable node ids. `graph_vector_index_rebuild/*` times the
maintenance rebuild that reclaims stale HNSW entries after vector update/delete
churn; fixture setup is excluded from the reported Criterion duration. Vector
benchmark IDs include a memory/cardinality suffix:
`m{index KiB}-{reachable KiB}_n{indexed rows}_e{HNSW entries}_l{live}_d{deleted}_g{links}`;
HNSW recall IDs encode exact-ID recall as `idbp{basis points}` and
tie-tolerant nearest-distance quality as `dqbp{basis points}` before that
memory suffix.
unindexed rows use `noidx`. Rebuild IDs add
`upd{updates}_del{deletes}_b{entries-live-deleted}_a{entries-live-deleted}_rk{reclaimed reachable KiB}`.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `graph_node_fetch` | 8.22 ns | 8.79 ns | 9.02 ns | Near-flat O(1) columnar fetch. |
| `graph_label_index_lookup` | 7.83 ns | 7.88 ns | 8.09 ns | Flat; `IStr`-keyed hash lookup. |
| `graph_typed_index_point` | 15.25 ns | 15.05 ns | 15.26 ns | Flat tri-state `lookup_eq`. |
| `graph_typed_index_range` | 7.05 µs | 37.89 µs | 55.74 µs | Sub-linear range scan. |
| `graph_composite_index_proxy` | 82.8 ns | 177.1 ns | 313.9 ns | Linear. |
| `graph_edge_create_cascade` | 362.9 µs | 747.4 µs | 1.481 ms | Mutation + commit body; teardown excluded. |
| `graph_mutation_commit_batch` (10) | 336.7 µs | 307.6 µs | 446.9 µs | Batched commit, 10 ops. |
| `graph_mutation_commit_batch` (100) | 408.2 µs | 420.8 µs | 552.7 µs | Batched commit, 100 ops. |
| `graph_mutation_commit_batch` (1000) | 952.4 µs | 1.053 ms | 1.226 ms | Batched commit, 1000 ops. |
| `graph_concurrent_reads` | 74.6 µs | 71.7 µs | 71.8 µs | ArcSwap snapshot read; flat above 10k. |
| `graph_bfs` (depth=1) | 106.3 ns | 109.0 ns | 109.6 ns | Depth-1 independent of N. |
| `graph_bfs` (depth=10) | 11.34 µs | 12.09 µs | 12.18 µs | Mostly traversal cost. |
| `graph_bfs` (depth=50) | 101.1 µs | 111.1 µs | 113.1 µs | Saturates ~110 µs. |

PR-local quick vector baseline:

| Bench | 1k | Notes |
|---|---:|---|
| `graph_exact_vector_scan/squared_euclidean_dim128_k10` | 46.6 µs (quick) | Exhaustive label-filtered scan over 1,000 vector nodes; scalar `f64` L2-squared accumulation; ~21.5 Melem/s. |
| `graph_vector_index_rebuild/hnsw_l2_dim128` | 182.4 ms (quick) | Rebuilds a 128-dim HNSW L2 index after 10% vector updates + 5% deletes; current M=18 1k quick ID `upd100_del50_b1100-950-150_a950-950-0_rk146` means 150 stale HNSW entries reclaimed and ~146 KiB reachable memory freed. |
| `graph_vector_index_rebuild/hnsw_cos_dim128` | 226.8 ms (quick) | Same rebuild fixture for 128-dim HNSW cosine, covering construction-side scorer reuse for metrics with bound query state. |

PR-local HNSW tuning spot-check:

| Bench | 10k | Notes |
|---|---:|---|
| `graph_hnsw_recall_validation/cluster_cos_d128_k10_ef10_idbp9875_dqbp9875` | 151.1 µs (quick) | `M=18` keeps both ID-overlap and distance-quality recall at 9875 bp on this 10k corpus while the separate duplicate-distance regression passes at 10000 bp; 10k row has 372,024 links and `m2647-7647`. Reserving HNSW search-layer heaps/visited sets moved the same row from 153.8 µs to 151.1 µs. Previous `M=16`: 144.2 µs / 330,688 links / `m2485-7485`; `M=24`: 187.7 µs / 496,032 links / `m3131-8131` and 10000 bp ID overlap. |

## §3 selene-graph — write pipeline & concurrency

Bench bins: `write_txn_lifecycle`, `provider_fanout`, `bound_type_validation`,
`concurrent_writers`, `graph_hub_delete`, `graph_read_under_write`.

### §3a Write-pipeline microbenches

`write_txn_lifecycle` create/delete rows below show the **batch axis at the 100k
fixture** (the headline scale); `empty_commit` shows the scale axis.

| Bench | Variant | Median | Notes |
|---|---|---:|---|
| `write_txn_lifecycle/empty_commit` | 10k / 50k / 100k | 211 / 139 / 270 µs | Empty-transaction commit floor. |
| `write_txn_lifecycle/create_only` @100k | batch 1 / 10 / 100 / 1000 | 342 µs / 360 µs / 469 µs / 1.18 ms | Isolated node create + commit. |
| `write_txn_lifecycle/delete_only` @100k | batch 1 / 10 / 100 / 1000 | 224 / 232 / 312 / 745 µs | Fixture seed excluded from timed body. |
| `provider_fanout/core_only` | providers=core | 258.7 µs | Commit-notification baseline. |
| `provider_fanout/extra_k1` / `k4` / `k16` | extra providers | 223.8 / 225.6 / 227.6 µs | No-op provider fanout — flat (notification is cheap). |
| `provider_fanout/extra_k4_with_error_one` | extra=4 + error | 227.4 µs | Error-path notification scaling. |
| `provider_fanout/extra_k4_with_panic_one` | extra=4 + panic | n/a | Opt-in `SELENE_BENCH_INCLUDE_PANIC_PROVIDER=1`. |
| `bound_type_validation/unbound_commit` | 10k / 50k / 100k | 291 / 246 / 320 µs | Commit without graph-type validation. |
| `bound_type_validation/bound_commit_simple` | 10k / 50k / 100k | 304 / 250 / 350 µs | Typed-commit validation delta (small). |
| `bound_type_validation/bound_commit_rich` | 10k / 50k / 100k | 1.01 / 1.14 / 1.67 ms | Wider type-graph validation delta. |
| `bound_type_validation/bound_schema_change` | 10k / 50k / 100k | 2.92 / 18.6 / 39.3 ms | Full graph-state revalidation; scales with N. |

### §3b `graph_hub_delete` — high-degree hub deletion (GRAPH-05 ✓ shipped)

Deleting a node cascades over every incident edge. GRAPH-05 made adjacency
removal **in place**: the deleted node's own `adjacency_out`/`adjacency_in`
entries are dropped wholesale (O(1) each) and each incident edge clears only the
neighbor side via `imbl::HashMap::get_mut` — no per-edge full-`SmallVec` clone.
That turned a degree-`D` hub delete from O(D²) to O(D); the curve below is now
linear (10× degree → ~9× time). This sweeps the **degree** axis (not node scale).

| Bench | degree=100 | degree=1000 | degree=10000 | Notes |
|---|---:|---:|---:|---|
| `graph_hub_delete` | 54.0 µs | 496 µs | 4.54 ms | Linear after GRAPH-05. Was 64.3 µs / 1.62 ms / 132.7 ms (O(D²)) — **30× faster at degree 10k**. |

### §3c `graph_read_under_write` — lock-free reads under contention (D10)

Times a fixed read batch (8 threads × 20k = 160k reads) while one background
writer churns commits on the `ArcSwap` snapshot. The D10 promise is that a held
write lock never blocks a reader; a regression that puts reads behind the write
lock collapses this. Dual of `concurrent_writers` (which times the writers).

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `graph_read_under_write` | 17.1 ms | 21.5 ms | 24.5 ms | ~107–153 ns/read; rises only with snapshot footprint, not lock contention. |

### §3d `concurrent_writers` — serialized writer queueing under contention

Thread fan-in arms sweep `[1, 2, 4, 8, 16, 32]` (representative `1/8/32` shown).
Two axes:

- **In-memory** (`threads{N}`, `threads{N}_with_readers8`) — no WAL; pure
  single-committer queueing + lock-free reads under contention. Group commit has
  nothing to coalesce here (no `fsync`), so it is not run on this axis.
- **WAL-backed** (`wal_threads{N}_batchOFF` vs `_batchON`) — a real on-disk WAL
  (tempdir per iteration; the committer is the sole `fsync` caller in
  `SyncPolicy::OnFlushOnly`). The only axis where group commit can win, because
  the win is coalesced `fsync` syscalls. `batchOFF` = `CommitBatching::Off` (one
  `fsync`/commit); `batchON` = `CommitBatching::DEFAULT_ON` (coalesce ≤64 commits
  / 8 MiB per `fsync`).

On `full`/`stress`, each WAL-backed arm also prints an **untimed**
`[concurrent_writers percentiles] … p50/p99/p999` line to stderr (the
tail-latency story the mean sample can't show).

| Bench | threads=1 | threads=8 | threads=32 | Notes |
|---|---:|---:|---:|---|
| `concurrent_writers/threads{N}` | 332 ms | 304 ms | 305 ms | In-memory; 1000 commits, 10 updates each. |
| `concurrent_writers/threads{N}_with_readers8` | 726 ms | 641 ms | 651 ms | Same load + 8 snapshot readers. |
| `concurrent_writers/wal_threads{N}_batchOFF` | 4.71 s | 3.86 s | 3.83 s | Real WAL, one `fsync`/commit. |
| `concurrent_writers/wal_threads{N}_batchON` | 4.57 s | 953 ms | **269 ms** | Group commit — **14× over batchOFF at 32 threads**; ≈ batchOFF at 1 thread (nothing to coalesce). |

## §4 selene-persist — WAL & snapshot

Bench bins: `wal`, `snapshot`, plus `graph_snapshot_roundtrip` (lives in the
`selene-graph` crate but exercises the persist/D14 path end to end).

### §4a WAL

`scale` = WAL entries, not graph nodes. `_no_fsync` rows use
`SyncPolicy::OnFlushOnly` (append/threshold/drop fsync suppressed; a caller
`flush()` would still sync).

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `persist_wal_append_single` | 65.2 ms | 322.4 ms | 630.9 ms | Single-entry loop, `EveryN(1000)`. |
| `persist_wal_append_single_no_fsync` | 11.5 ms | 55.7 ms | 111.2 ms | Donor-parity diagnostic, no append fsync. |
| `persist_wal_append_batch_1000` | 6.49 ms | 9.57 ms | 12.58 ms | 1000-change entries — **50× faster than per-entry at 100k**. |
| `persist_wal_append_batch_1000_no_fsync` | 2.04 ms | 5.04 ms | 8.28 ms | Batched, no flush in timed body. |
| `persist_wal_replay` | 4.23 ms | 18.67 ms | 32.27 ms | Fixed-layout header + xxh3 + BufReader. |

#### `persist_wal_body_size_no_fsync` — entry-body packing (PERSIST-04)

Fixed total changes (100k), swept changes-per-entry packing — isolates the
per-byte serialize+write cost from the per-entry overhead the count sweeps
cover. Per-entry overhead dominates at small bodies; the minimum is ~10k
changes/entry, after which large-`Vec` build/alloc creeps back in. This was the
PERSIST-04 measurement surface; the stable manual `write_vectored` candidate was
measured-rejected on 2026-06-01 because it regressed the WAL append hot path, so
the contiguous `Vec` + `write_all` path remains the baseline.

| Bench | per-entry=100 | =1000 | =10000 | =50000 | Notes |
|---|---:|---:|---:|---:|---|
| `persist_wal_body_size_no_fsync` | 12.5 ms | 8.42 ms | 7.22 ms | 13.1 ms | Equal total work; U-shaped in packing; vectored write rejected. |

#### `persist_wal_sync_sweep` — sync-policy sweep

Append + explicit `flush()` across sync policies. The fsync-frequent policies
(`every1`/`every10`/`every100`) are bound by `fsync` syscall latency, not
selene-db code, and balloon to tens of seconds at 100k — they are **capped at
≤10k** so a full sweep is not dominated by one durability cell.

| Bench | 1k | 10k | 100k | Notes |
|---|---:|---:|---:|---|
| `persist_wal_sync_sweep/every1` | 3.74 s | 39.5 s | n/a (capped) | `EveryN(1)` — fsync per entry. |
| `persist_wal_sync_sweep/every10` | 378 ms | 3.99 s | n/a (capped) | `EveryN(10)`. |
| `persist_wal_sync_sweep/every100` | 47.5 ms | 479 ms | n/a (capped) | `EveryN(100)`. |
| `persist_wal_sync_sweep/every1000` | 7.79 ms | 65.9 ms | 655 ms | `EveryN(1000)`. |
| `persist_wal_sync_sweep/on_flush_only` | 7.60 ms | 15.8 ms | 113 ms | `OnFlushOnly` + caller flush. |

### §4b Snapshot

`persist_snapshot_*` measure the SLSN **container** (framing + per-section zstd +
body hash) over synthetic byte payloads. `scale` drives section bytes.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `persist_snapshot_write` | 341.9 µs | 444.5 µs | 572.0 µs | Five independently-compressed sections. |
| `persist_snapshot_read` | 278.9 µs | 425.5 µs | 605.0 µs | Snapshot read-and-apply. |
| `persist_full_recovery` | 3.01 ms | 11.28 ms | 20.75 ms | Snapshot reconcile + WAL replay. |

### §4c `graph_snapshot_roundtrip` — real rkyv graph encode/decode (D14)

Unlike the synthetic-bytes snapshot bench above, this drives the **real**
`CoreProvider` path over fixture rows: `IndexProvider::write_section` over every
`CORE/*` sub-tag (rkyv archive of `CORE/NODE`+`CORE/EDGE` positional rows, D14),
then a recovery-mode provider + `finish_recovery` (positional placement / id↔row
rebuild). Self-validating: asserts node/edge counts survive the roundtrip once
(untimed) before measuring. `scale` = fixture node count.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `graph_snapshot_roundtrip/encode` | 5.03 ms | 31.2 ms | 69.2 ms | rkyv encode of all `CORE/*` sections. |
| `graph_snapshot_roundtrip/decode` | 20.1 ms | 106.3 ms | 216.4 ms | Positional recovery + `finish_recovery` — dominates. |
| `graph_snapshot_roundtrip/roundtrip` | 26.2 ms | 141.2 ms | 289.9 ms | End-to-end (≈ encode + decode). |

## §5 selene-gql — parse / plan / execute

Bench bins: `parse`, `analyze`, `plan_optimize`, `expression_eval`,
`procedure_call_repeat`, `correlated_subquery`, `write_e2e`. The first four are
scale-independent (single-query CPU).

| Bench | Median | Notes |
|---|---:|---|
| `gql_parse_corpus/m5c` | 879.6 µs | Full single-query parse-corpus latency. |
| `gql_parse_hostile/bracket_artifacts` | 566 ns | DoS-hardening: pathological `[`-backtracking input. |
| `gql_parse_hostile/recursion_chains` | 12.6 µs | DoS-hardening: deep sign/NOT/CASE recursion-guard input. |
| `gql_analyze_corpus/m5c` | 21.98 µs | Semantic analysis. |
| `gql_plan_optimize_corpus/m5c` | 48.13 µs | Planner/optimizer end-to-end. |
| `gql_plan_ir_clone/representative` | 164.0 ns | IR-clone hot path. |
| `gql_expression_eval/*` (9 cases) | 180–245 ns | Scalar eval: predicates, scalar fns, CASE, list access, binary ops. |
| `procedure_call_repeat/no_cache` | 2.958 ms | 100 short-lived sessions, parse/analyze/plan each. |
| `procedure_call_repeat/shared_cache` | 27.49 µs | Shared `Arc<CallPlanCache>` warm-hit — **99.1% lower**. |

PR-local quick vector procedure baseline:

| Bench | Median | Notes |
|---|---:|---|
| `procedure_vector_search/shared_cache_squared_euclidean_dim128_k10_1000` | 46.6 µs (quick) | Cached `CALL selene.vector_search_nodes` over 1,000 vector nodes; scalar exact scan, ~21.4 Melem/s. |

### §5a `gql_correlated_subquery` — correlated EXISTS/COUNT execution (GQLRT-05)

The only read-query **execution** bench in the suite (`expression_eval` is
scalar-only; `write_e2e` is write-only). A correlated subquery is re-evaluated
per outer row and its pattern schema is rebuilt per row (`schema_for_pattern`); a
memoization win (GQLRT-05) would otherwise be invisible. In-memory graph (no WAL)
so the per-row schema rebuild dominates, not durability. Uses a **small scale
envelope** (2.5k/5k/10k fixture rows, ~scale/3 `Person` rows) — correlated
re-evaluation is O(rows × subquery), so the cost grows super-linearly and 50k/100k
would be a multi-minute single arm.

_Refreshed post-GQLRT-05 (an A/B of development HEAD vs the feature branch on this
M5, profile `full`), so these run ahead of the `3a864ac` header until the next
clean re-sweep. The per-statement target-schema memo improved every arm: exists
−1.8 / −6.8 / −4.7 %, count −3.7 / −5.8 / −0.4 % at 2.5k / 5k / 10k. The 2.5k–5k
arms are clearly significant (p<0.05); the 10k arms are noisier (the 10k/count
−0.4% is within run-to-run noise). A modest win — the per-row `schema_for_pattern`
rebuild is a real but minority fraction of subquery cost, dominated by the inner
MATCH._

| Bench | 2.5k | 5k | 10k | Notes |
|---|---:|---:|---:|---|
| `gql_correlated_subquery/exists` | 62.2 ms | 252.9 ms | 1.008 s | `FILTER EXISTS { (p)-[:KNOWS]->(:Person) }`; schema memo (GQLRT-05). |
| `gql_correlated_subquery/count` | 62.3 ms | 253.3 ms | 1.005 s | `COUNT { (p)-[:KNOWS]->(:Person) }` projection. |

### §5b `write_e2e` — GQL write end-to-end

Two families. The **in-memory CPU** family runs on a no-WAL `SharedGraph` to
isolate parse/plan/execute + in-memory commit CPU. The **durable** family
(`*_with_flush`, `direct_*`) keeps a real WAL on `OnFlushOnly` /
`CommitBatching::Off`. The `match_*` / `insert_node_with_edge` arms scan the
fixture and so scale with N; the single-node arms are flat.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `write_e2e/gql_insert_single_node_per_iter_plan` | 317 µs | 237 µs | 394 µs | Parse/plan/execute per iter (in-memory). |
| `write_e2e/gql_insert_single_node_preplanned` | 279 µs | 190 µs | 342 µs | Preplanned single-node insert. |
| `write_e2e/gql_insert_single_node_cached` | 129 µs | 115 µs | 151 µs | Plan-cache warm hit. |
| `write_e2e/gql_insert_single_node_cached_with_schema_churn` | 152 µs | 165 µs | 289 µs | Cache hit under schema churn. |
| `write_e2e/gql_insert_node_with_edge_preplanned` | 1.67 ms | 11.75 ms | 24.86 ms | Preplanned insert + matched source + edge (scans). |
| `write_e2e/gql_match_set_preplanned` | 1.76 ms | 11.74 ms | 24.12 ms | Indexed match + property update (scans). |
| `write_e2e/gql_match_delete_preplanned` | 1.68 ms | 12.25 ms | 24.97 ms | Fresh fixture per iter (target deleted). |
| `write_e2e/gql_multi_statement_txn_preplanned` | 280 µs | 191 µs | 350 µs | START, three INSERTs, COMMIT. |
| `write_e2e/explicit_txn_3_inserts_rust_api` | 275 µs | 223 µs | 363 µs | Three inserts via the Rust txn API. |
| `write_e2e/explicit_txn_3_inserts_rollback` | 279 µs | 198 µs | 355 µs | Same, rolled back. |
| `write_e2e/gql_insert_single_node_preplanned_with_flush` | 4.22 ms | 4.27 ms | 3.95 ms | Durable: preplanned insert + WAL flush. |
| `write_e2e/direct_insert_single_node_with_wal_flush` | 4.20 ms | 4.30 ms | 4.17 ms | Direct mutation + one WAL flush. |
| `write_e2e/direct_insert_single_node_with_wal_flush_every10` | 30.5 ms | 32.2 ms | 32.4 ms | Ten direct inserts over one flush. |

## §6 selene-algorithms

Bench bins: `algo_bench`, `projection`. Fixture: `BenchFixture::build(N)` (≈3N
edges) for pagerank/betweenness/apsp and projection; `planted_community_graph(N)`
(≈6N edges, ~N/64 communities) for triangle_count and louvain.

### §6a Algorithm baselines (Sequential vs Auto)

| Bench | Scale | Sequential | Auto | Notes |
|---|---:|---:|---:|---|
| `algo/pagerank` | 10k | 87.9 µs | 247.0 µs | Sparse graph: Auto pays coordination overhead… |
| `algo/pagerank` | 50k | 494.5 µs | 687.3 µs | …at every scale on this fixture. |
| `algo/pagerank` | 100k | 1.035 ms | 1.284 ms | Auto closes the gap on denser graphs; API exposes both. |
| `algo/betweenness` | 10k | 25.52 ms | 7.73 ms | **3.3× Auto** — endpoint-aware sampling. |
| `algo/betweenness` | 50k | 135.3 ms | 44.95 ms | **3.0× Auto** — per-source SSSP parallelizes. |
| `algo/betweenness` | 100k | 266.1 ms | 101.7 ms | **2.6× Auto** — headline rayon win. |
| `algo/triangle_count` | 10k | 631.9 µs | 620.6 µs | Tiny; already efficient sequentially. |
| `algo/triangle_count` | 50k | 3.253 ms | 2.474 ms | 1.3× Auto. |
| `algo/triangle_count` | 100k | 6.520 ms | 4.960 ms | 1.3× Auto. |
| `algo/apsp` | 200 | 621.8 µs | 306.5 µs | All-pairs SSSP; scale = source count. |
| `algo/apsp` | 500 | 4.091 ms | 1.457 ms | 2.8× Auto. |
| `algo/apsp` | 1k | 17.17 ms | 5.576 ms | **3.1× Auto** — strong scaling at 10 cores. |
| `algo/louvain` | 10k | 1.723 ms | n/a | Sequential-only (V170). |
| `algo/louvain` | 50k | 9.294 ms | n/a | |
| `algo/louvain` | 100k | 19.31 ms | n/a | |

### §6b `projection` — CSR foundation (ALGO-01/02/05)

Every algorithm runs *over* a projection, but `algo_bench` builds it in untimed
setup. This isolates the build (graph scan + CSR construction) and the per-edge
neighbor walk — exactly the two numbers the CSR dense-`u32` reshape
(ALGO-01/02/05) changes.

_§6a/§6b medians below are refreshed post-ALGO-01/02/05 (an A/B of development
HEAD vs the feature branch on this M5, profile `full`), so they run ahead of the
`3a864ac` north-star header until the next clean re-sweep. The dense-`u32` cache
trades a one-time **+4–7% `projection_build`** (one extra `u32` write per
neighbor, `ProjNeighbor` 24→32 B) for **−6 to −52%** across every algorithm that
resolved the dense index per edge (pagerank/louvain/apsp/triangle); even raw
`neighbor_iter` dropped −4 to −6%._

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `algo/projection_build` | 1.338 ms | 16.10 ms | 41.50 ms | Full `GraphProjection::build`; +4–7% (the `dense:u32` write). |
| `algo/projection_neighbor_iter` | 20.7 µs | 128.0 µs | 291.9 µs | Sweep every node's out-neighbor slice. |

## Cluster-B regression targets

This doc is the baseline for the v1.2 cluster-B performance-uplift work
(graph node 767). Each optimization has a dedicated bench whose median should
move when it lands; refresh that row + diff against the `northstar` baseline to
confirm the win and guard the surrounding rows against regression.

| Target | Optimization | Watch bench | Current baseline |
|---|---|---|---|
| CORE-06 ✓ | Box `Value` `Path` + time variants (shrink `size_of`) | `core_value_clone/*` + `size_of::<Value>` stderr | **32 B** (was 128); vec 4.62 µs / pmap 53.8 ns |
| GRAPH-05 ✓ | In-place adjacency delete O(D²)→O(D) | `graph_hub_delete` (now linear) | **4.54 ms** @ degree 10k (was 133 ms — 30×) |
| PERSIST-04 rejected | WAL vectored write regressed append on Darwin/macOS | `persist_wal_body_size_no_fsync` (large-body arms) | measured-rejected 2026-06-01; keep contiguous `Vec` + `write_all` |
| ALGO-01/02/05 ✓ | CSR dense-`u32` cache on `ProjNeighbor` | `algo/projection_build` + `…_neighbor_iter` + algo medians | **pagerank −15..31% · louvain −23..26% · apsp −9..52% · triangle −6..11% · iter −4..6%**; build +4–7% one-time (24→32 B/neighbor) |
| GQLRT-05 ✓ | Memoize correlated-subquery target schema (per statement, by expr id) | `gql_correlated_subquery/{exists,count}` | **−2 to −7%** — memo elides the per-row `schema_for_pattern` walk |
| D10 (guard) | Lock-free reads stay flat under writes | `graph_read_under_write` | 24.5 ms @100k |
| D14 (guard) | Snapshot rkyv encode/positional recovery | `graph_snapshot_roundtrip/{encode,decode}` | enc 69 ms / dec 216 ms @100k |

## Update protocol

1. From a clean, synced `development` on a **quiet machine** (background load
   pollutes medians): `git checkout development && git pull --ff-only`.
2. Run the full sweep, saving the baseline:
   `scripts/run-benches.sh --profile full --save-baseline northstar`.
3. Refresh the header: `_Last measured_` date + hardware footprint (capture
   commands above) + `git rev-parse --short HEAD`.
4. Fill every `Median` / `Sequential` / `Auto` cell from criterion stdout.
5. Commit: `chore(bench): refresh BENCHMARKS.md (<short hardware string>)`.

## Out of scope

- CI bench jobs — benchmarks are local-only and sequential (CI only lints the
  invocation hygiene; it never executes benches).
- Cross-host comparison automation.
- Donor regression-target comparison (lives in gitignored
  `_design/perf-baselines.md`).
