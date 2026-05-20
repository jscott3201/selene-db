# Perf baselines — donor SeleneDB measurements as regression targets

> Tracked counterpart at `/BENCHMARKS.md`. This file remains donor regression targets.

Status: **draft (2026-05-08)**
Pairs with: spec 10 (testing strategy), `project_donor_perf_audit.md` (memory).
Source: audit of `/Users/justin/Development/SeleneDB/` on 2026-05-08, criterion 0.8 medians on M-class CPU.

## 1. Purpose

Capture the donor's measured numbers as **regression baselines** for selene-db. The donor accumulated these via real benchmark cycles on a working implementation; selene-db should match or beat each one.

## 2. Two layers of regression gate

| Layer | Tool | Purpose |
|---|---|---|
| **CI gate (durable)** | `iai-callgrind 0.16` | Instruction-count gates that survive noisy runners. **Locked baselines.** A regression here blocks merge. |
| **Local / nightly (informational)** | `criterion 0.8` | Wall-clock medians for human-readable progress; varies with hardware. **Reported, not gated.** |

Wall-clock numbers below are donor-machine medians. Treat them as targets selene-db must come within ~20% of on equivalent hardware. The durable form is the iai-callgrind instruction count we lock in once selene-db's bench targets exist.

## 3. Per-axis baselines

### 3.1 selene-graph — single-graph hot paths

Owner crate: `crates/selene-graph/benches/graph_bench.rs` (donor source: `crates/selene-graph/benches/graph_bench.rs`).

| Operation | Scale 200 | Scale 1k | Scale 10k | Priority |
|---|---:|---:|---:|---|
| Single-node fetch | 1.2 µs | 5.8 µs | 62 µs | P0 |
| Label index lookup | 152 ns | 617 ns | 6.1 µs | P0 |
| TypedIndex lookup | **2.1 ns** | **2.1 ns** | **2.1 ns** (flat) | P0 |
| TypedIndex iter_asc | **929 ps** | **929 ps** | **929 ps** (flat) | P0 |
| CompositeTypedIndex lookup | 85 ns (flat) | 85 ns | 85 ns | P0 |
| Edge create + node remove (cascade) | — | 402 µs | ~ sub-linear | P0 |
| BFS d=1 | — | — | — (donor unbenched) | P1 |
| BFS d=10 | — | 84.3 µs | 891 µs | P0 |
| BFS d=50 | — | — | — | P1 |
| Mutation commit batch=10 | — | — | — | P0 |
| Mutation commit batch=100 | 571 µs | 571 µs | 571 µs | P0 |
| Mutation commit batch=1000 | — | — | 3.07 ms | P0 |
| Concurrent reads (10 threads) | ~75 µs | ~75 µs | ~75 µs (**flat**) | P0 |

The **flat lines** are the load-bearing donor proofs:
- TypedIndex's BTree-over-RoaringBitmap shape is O(log n) with tiny constants — the lookup is dominated by hash + dispatch, not data size.
- ArcSwap concurrent reads are O(1) regardless of graph size; if selene-db sees a non-flat curve here, the snapshot mechanism is wrong.

### 3.2 selene-persist — durability hot paths

Owner crate: `crates/selene-persist/benches/persist_bench.rs` (donor source: `crates/selene-persist/benches/persist_bench.rs`).

| Operation | Scale 200 | Scale 1k | Scale 10k | Priority |
|---|---:|---:|---:|---|
| WAL append (single, fsync per entry) | — | — | 11.4 ms | P0 |
| WAL append_batch (group commit) | — | — | 1.99 ms (5.7× single) | P0 |
| WAL replay | — | — | 9.20 ms | P0 |
| Snapshot read (recovery body load) | — | — | 1.93 ms | P0 |
| Full recovery (snap + WAL replay) | — | — | 5.87 ms | P0 |
| Frame decode header | 1.0 ns | 1.0 ns | 1.1 ns | P0 |
| Postcard NodeDto serialize | 161 ns | — | — | P0 |
| Postcard NodeDto deserialize | 123 ns | — | — | P0 |
| Zstd 1 KB compress | 699 ns | — | — | P0 |
| Zstd 1 KB decompress | 178 ns | — | — | P0 |

selene-db should **beat** snapshot read (1.93 ms at 10k) — D14's rkyv mmap path is structurally faster than the donor's postcard snapshot deserialize, so a regression here means rkyv didn't pay off.

### 3.3 selene-gql — language hot paths

Owner crate: `crates/selene-gql/benches/*.rs` (donor source: equivalent benches under selene-server / selene-gql).

| Operation | Scale | Donor median | Priority |
|---|---|---:|---|
| Plan cache hit | n/a | **20 ns** | P0 |
| Plan cache miss (parse + plan, simple) | n/a | TBD | P0 |
| Insert single node (e2e GQL parse + plan + mutate + WAL) | 10k | 54.8 µs | P0 |
| INSERT via gql_query MCP path | 10k | ~686 µs | P2 (server-parity only; selene-db is library-only) |
| count_star | 10k | 3.11 ms | P0 |
| ORDER BY + LIMIT (TopK) | 10k | 3.47 ms | P0 |
| **EXISTS semijoin (regression-fix floor)** | 10k | **<1 ms** | P0 |

The **EXISTS line is the regression-never-let-return**: pre-fix was 33 ms, post-fix <1 ms. A selene-db EXISTS implementation that returns to the 33 ms shape means outer-binding correlation regressed; the donor's lesson is in `project_donor_perf_audit.md`.

### 3.4 selene-vector — HNSW hot paths

Owner crate: `crates/selene-vector/benches/*.rs` (donor source: `crates/selene-graph/benches/hnsw_bench.rs`).

| Operation | Scale 1k | Scale 10k | Priority |
|---|---:|---:|---|
| dot_product (384-dim) | 119 ns | 119 ns | P1 |
| cosine_similarity (384-dim) | 345 ns | 345 ns | P1 |
| HNSW search top-10 | 140 µs | 241 µs | P1 |
| HNSW build | **1.31 s** | scaling factor TBD | P1 (the donor's red flag — beat it) |
| HNSW incremental insert | TBD | TBD | P1 |
| Recall (target 95%, dim=384, M=16) | TBD | TBD | P1 |

NON-PARITY selene-db BRIEF-91 measurement: `vector_hnsw_build` is 71.04 ms @ n=1k and 578.2 ms @ n=5k on Apple M5 (`BENCHMARKS.md §7e`, commit `11537ed`). This uses dim=16, M=8, ef_construction=64, L2; donor dim/M for the 1.31 s row is underdocumented, so this is directional only.

The **HNSW build at 1k = 1.31 s** is the donor's red flag — it indicated the build path was the perf bottleneck. simsimd kernels (D9) + filter-aware traversal (D5 amendment NaviX) should both contribute to beating it.

### 3.5 Cross-crate algorithm baselines

Owner crate: `crates/selene-algorithms/benches/*.rs`.
Status: M12 closed with Criterion wall-clock surfaces for Sequential and Auto modes on PageRank, betweenness, triangle_count, and APSP. Louvain remains sequential-only in v1.0. Medians remain TBD until the v1.x local measurement pass.

| Algorithm | Modes | Scale 200 | Scale 500 | Scale 1k | Scale 10k | Priority |
|---|---|---|---|---|---|---|
| PageRank | Sequential / Auto | TBD / TBD | — | TBD / TBD | TBD / TBD | P1 |
| Betweenness | Sequential / Auto | TBD / TBD | — | TBD / TBD | TBD / TBD | P1 |
| Triangle count | Sequential / Auto | TBD / TBD | — | TBD / TBD | TBD / TBD | P1 |
| APSP | Sequential / Auto | TBD / TBD | TBD / TBD | TBD / TBD | capped | P1 |
| Louvain | Sequential only | TBD | — | TBD | TBD | P2 |

Donor measured graph algorithms but specific medians weren't surfaced in the audit. M12 adds the bench harness with TBD cells only; do not commit local Criterion medians until the dedicated v1.x measurement pass.

### 3.5.1 selene-algorithms-pack — GQL CALL adapter overhead

Owner crate: `crates/selene-algorithms-pack/benches/algo_pack.rs`.
Invocation: `scripts/run-benches.sh --profile quick --layer criterion --filter algo_pack`.
Status: M10.B6 seeded infrastructure; BRIEF-117 captured quick-profile Criterion medians on 2026-05-20 before adding cancellation checkpoints.

| Bench | Fixture | Median | Notes |
|---|---|---:|---|
| `algo_pack/projection_build_default` | 1k deterministic directed graph | 229.81 us | Times `CALL algo.projection_build('p', NULL, NULL, NULL)`. |
| `algo_pack/algo_pagerank_default` | 256-node prebuilt projection | 141.31 us | Adapter + projection lookup + PageRank row projection. |
| `algo_pack/algo_dijkstra_single_pair` | 256-node prebuilt projection with one Source/Target pair | 38.268 us | Adapter + typed NodeRef MATCH bindings + Dijkstra row projection. |
| `algo_pack/algo_apsp_default` | 96-node prebuilt projection | 1.4773 ms | Small-N APSP to avoid local smoke-test cliffs. |
| `algo_pack/algo_betweenness_default` | 256-node prebuilt projection | 318.27 us | Exact betweenness mode (`sample_size = NULL`). |
| `algo_pack/algo_louvain_default` | 256-node prebuilt projection | 90.705 us | Louvain adapter row projection. |
| `algo_pack/algo_triangle_count_default` | 256-node prebuilt projection | 121.55 us | Triangle-count adapter row projection. |
| `algo_pack/algo_label_propagation_default` | 256-node prebuilt projection | 59.633 us | Label-propagation adapter row projection. |

### 3.5.2 selene-vector-pack — GQL CALL adapter overhead

Owner crate: `crates/selene-vector-pack/benches/vector_pack.rs`.
Invocation: `scripts/run-benches.sh --profile quick --layer criterion --filter vector_pack`.
Status: M11.B5 seeds infrastructure only; medians are TBD for a v1.x local measurement pass.

| Bench | Fixture | Median | Notes |
|---|---|---:|---|
| `vector_pack/search_default` | 1k HNSW index | TBD | Times `CALL vector.search('default', ...)` over a prebuilt index. |
| `vector_pack/upsert_default` | single-node clean HNSW graph per iteration | TBD | Uses `iter_batched` so each iteration starts from an empty index. |
| `vector_pack/bulk_upsert_default` | 100-node clean HNSW graph per iteration | TBD | Measures one 100-row `vector.bulk_upsert` procedure dispatch. |
| `vector_pack/ivf_search_default` | 256-row trained IVF index | TBD | Fixture uses explicit small `IvfConfig` plus CQNT/IPQB/POST training writes. |
| `vector_pack/ivf_bulk_upsert_default` | 100-node clean IVF graph per iteration | TBD | Measures one 100-row `vector.ivf_bulk_upsert` procedure dispatch. |
| `vector_pack/ivf_stats_default` | 256-row trained IVF index | TBD | Measures flat-row `vector.ivf_stats` projection. |

### 3.6 Allocator policy

Bench binaries use `mimalloc 0.1` as the `#[global_allocator]` (see INFRA-04). Library crates are allocator-agnostic: embedders pick their own allocator. Captured iai-callgrind baselines reflect mimalloc; comparing local runs against them requires the same allocator.

If a bench is rerun with a different allocator (system, jemalloc, snmalloc), the iai-callgrind comparison can report allocator artifacts as regressions. Re-baseline before reporting.

Why mimalloc:
- Microsoft's [mimalloc benchmarks](https://microsoft.github.io/mimalloc/bench.html) show consistent wins on small-allocation-heavy workloads.
- [rust-analyzer's adoption PR #19603](https://github.com/rust-lang/rust-analyzer/pull/19603) is the closest-shape production case study: a Rust binary that is allocation-heavy and latency-sensitive.
- Smaller memory footprint than jemalloc on this workload class matters for LDBC SF1+ baselines.
- It stays Rust-friendly via the `mimalloc` crate without requiring jemalloc-style C build tooling on macOS.

## 4. Bench profile envelopes

The donor used three env-var-controlled profiles (`crates/selene-testing/src/bench_profiles.rs`) so CI runs the cheap profile and developers can opt into the expensive one. Mirror in selene-db:

| Profile | Scales | Iterations | Use |
|---|---|---|---|
| `quick` | 200 | reduced | Default for CI iai-callgrind gates. |
| `full` | 200, 1k, 10k | criterion default | Local development. |
| `stress` | 200, 1k, 10k, 100k | criterion default | Nightly perf-tracking job. |

Profile is selected by `SELENE_BENCH_PROFILE` env var. Default is `quick`.

## 5. CI gate plan (rollout order)

The iai-callgrind gates land as benchmarks come online. Per spec 10's milestone phasing:

1. **M3 close — selene-persist baselines.** WAL append / replay / snapshot read / full recovery. (Already shipped via BRIEFs 10/11/12; benches still TBD.)
2. **M4 close — selene-graph baselines.** Single-node fetch, label index lookup, TypedIndex, BFS, mutation commit, concurrent reads.
3. **M5d close — selene-gql baselines.** Plan cache, parse + plan, EXISTS semijoin, count_star, ORDER BY + LIMIT.
4. **M6+ — selene-algorithms / selene-vector baselines.** HNSW, distance kernels, graph algorithms.

Each gate locks an iai-callgrind instruction count once the bench is stable; criterion wall-clock is reported in CI logs but not gated.

## 6. Open questions

- **Hardware target for criterion baselines.** Donor numbers come from M-class Apple silicon. CI runs on x86 GitHub Actions; the iai-callgrind instruction-count form is the right durable answer because it's hardware-independent. The wall-clock targets here are aspirational on equivalent hardware.
- **Recall target for HNSW.** Donor didn't surface a single recall number; the v1.0 target is "configurable per-query target-recall API" per D5 amendment NaviX. Need to fix a default before locking the bench.
- **LDBC SNB SF1 / SF10 baselines.** Not in the donor (donor never targeted LDBC). selene-db will measure and lock these in M5e (conformance/LDBC milestone) without a reference number to chase.
- **Allocator selection.** Settled by INFRA-04: bench binaries standardize on `mimalloc 0.1`; library crates remain allocator-agnostic.

## 7. References

- `project_donor_perf_audit.md` (auto-memory) — the audit findings these numbers come from.
- Donor `Benchmarks.md` and `crates/*/benches/*.rs` — primary sources.
- D9 (CLAUDE.md) — `iai-callgrind 0.16` and `criterion 0.8` are the workspace-pinned bench tools.
- spec 10 — milestone-phased CI plan that consumes these baselines.

## 8. Measured selene-db baselines

### 8.1 Run: 2026-05-11 post-M5f closure

**Platform:** Apple Silicon (macOS); criterion quick profile (`--profile quick --layer criterion`); mimalloc allocator (per §3.6). iai-callgrind layer skipped (valgrind not available on macOS; runner degrades gracefully per BRIEF-56-companion CI cleanup). Re-run on Linux to capture iai instruction counts.

| Crate / bench | Scale | Median | Notes |
|---|---:|---:|---|
| selene-graph: graph_node_fetch | 1000 | 4.33 ns | Donor target 5.8 µs at 1k — selene-db is ~1300× faster. |
| selene-graph: graph_label_index_lookup | 1000 | 4.16 ns | Donor target 617 ns at 1k. |
| selene-graph: graph_typed_index_point | 1000 | 17.17 ns | Donor target 2.1 ns (flat). Selene-db ~8× slower; investigate. |
| selene-graph: graph_typed_index_range | 1000 | 1.31 µs | Range query; no donor equivalent. |
| selene-graph: graph_composite_index_proxy | 1000 | 45.11 ns | Donor target 85 ns (flat). |
| selene-graph: graph_edge_create_cascade | 1000 | 362 µs | Donor target 402 µs. ✓ |
| selene-graph: graph_mutation_commit_batch | 1000 / batch=10 | 317 µs | — |
| selene-graph: graph_mutation_commit_batch | 1000 / batch=100 | 353 µs | Donor target 571 µs. ✓ |
| selene-graph: graph_mutation_commit_batch | 1000 / batch=1000 | 665 µs | — |
| selene-graph: graph_concurrent_reads | 1000 | 75.4 µs | Donor target ~75 µs (flat). ✓ |
| selene-graph: graph_bfs/d1 | 1000 | 96 ns | — |
| selene-graph: graph_bfs/d10 | 1000 | 10.14 µs | Donor target 84.3 µs at 1k. ✓ ~8× faster. |
| selene-graph: graph_bfs/d50 | 1000 | 68.34 µs | — |
| selene-persist: wal_append_single | 1000 | 6.49 ms | Donor target 11.4 ms at 10k; not directly comparable. |
| selene-persist: wal_append_batch_1000 | 1000 | 3.86 ms | Group-commit win. |
| selene-persist: wal_replay | 1000 | 5.73 ms | Donor 9.20 ms at 10k. |
| selene-persist: snapshot_write | 1000 | 382.92 µs | — |
| selene-persist: snapshot_read | 1000 | 141.27 µs | Donor target 1.93 ms at 10k → rkyv mmap (D14) paid off. ✓ |
| selene-persist: full_recovery | 1000 | 3.53 ms | Donor 5.87 ms at 10k. |
| selene-gql: parse_corpus/m5c | corpus | 266.14 µs | M5c plan-corpus parse. |
| selene-gql: analyze_corpus/m5c | corpus | 5.29 µs | Analyze pass. |
| selene-gql: plan_optimize_corpus/m5c | corpus | 18.37 µs | Plan + optimize. |
| selene-gql: plan_ir_clone | representative | 98.42 ns | Plan IR clone hot path. |

**Flags worth tracing:**
- `graph_typed_index_point` at 17 ns is 8× donor's flat 2.1 ns line. Donor used a BTree-over-RoaringBitmap shape; verify ours matches and the hash-dispatch overhead isn't dominating at 1k scale.
- `graph_node_fetch` improvement (1300× faster) is suspicious — verify the bench isn't being optimized away (black-box). If real, the simpler imbl property store is the win.

### 8.2 Selene-algorithms baselines

Not yet measured. selene-algorithms doesn't ship Criterion benches today (M5f closed without bench harness). Add benches in a future brief; the §3.5 cross-crate algorithm baselines table will populate then.

### 8.3 Selene-pack baselines

Not yet measured. selene-pack has bench scaffolding (manifest parse, hash compute, activation) but no committed baselines.
