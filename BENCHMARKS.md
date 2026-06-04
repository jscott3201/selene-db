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
run-benches.sh --bench single_graph --filter graph_exact_vector_scan --vector-scales million
run-benches.sh --bench vector_index_rebuild --vector-scales 10000,50000
run-benches.sh --bench vector_index_rebuild --filter graph_vector_index_rebuild/ivf --vector-scales 100000
SELENE_VECTOR_IVF_INSERT_DRIFT_BPS=100,500,1000 run-benches.sh --bench vector_ivf_insert_drift --vector-scales 10000
run-benches.sh --bench vector_index_rebuild --allocator system   # allocator A/B without mimalloc
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

Vector benches also accept an independent runner override with
`--vector-scales`. The flag exports both `SELENE_VECTOR_BENCH_SCALES` and
`SELENE_VECTOR_REBUILD_BENCH_SCALES`, so it covers exact/ANN query sweeps and
index rebuild sweeps without changing non-vector benches.

`vector_ivf_insert_drift` also accepts
`SELENE_VECTOR_IVF_INSERT_DRIFT_BPS` as a comma-separated basis-point sweep over
post-training novel inserts. The default remains `1000` (10%) so routine runs
keep the historical row count; use `100,500,1000` for the maintenance-policy
threshold sweep.

For `vector_index_rebuild`, Criterion's positional `--filter` is mirrored into
`SELENE_VECTOR_REBUILD_GROUP_FILTER` and
`SELENE_VECTOR_REBUILD_VARIANT_FILTER=ivf|hnsw` when the filter names a rebuild
group or ANN family. Those prefilters skip irrelevant fixture construction, so
focused large-scale IVF or HNSW runs do not pay preview costs for unrelated
groups or the other family.

| Vector scale selector | Scales | Use |
|---|---|---|
| `quick` | 1k | fast vector smoke |
| `full` | 10k / 50k / 100k | publish-quality vector sweep |
| `stress` | 1k / 10k / 50k / 100k / 250k | opt-in stress sweep |
| `large` | 250k / 1M | local large-scale validation |
| `million` | 1M | focused million-vector run |
| comma list | sorted positive integers | custom A/B scale envelope |

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

All committed benchmark rows use `mimalloc` as the global allocator; the library
crates are allocator-agnostic. For allocator A/B work, run the same scoped bench
twice with `--allocator mimalloc` (the default) and `--allocator system`.

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
future ANN layer; current kernels use safe `wide::f64x4` accumulation over the
existing `f64` score semantics, so these rows are the SIMD/Rayon improvement
tripwire.

| Bench | Median | Notes |
|---|---:|---|
| `core_value_clone/vec_mixed_1024` | 4.62 µs | Clone a 1024-element mixed-variant `Vec<Value>`. **−25%** vs the 128 B layout (was 6.10 µs). |
| `core_value_clone/property_map_5` | 53.8 ns | Clone a 5-key `PropertyMap` (Int/Float/String/Duration/ZonedDateTime). **−29%** (was 74.6 ns) — the *worst* case: 2 of 5 keys are now boxed (alloc-on-clone), so non-temporal maps gain more. |
| `core_value_clone/property_map_from_pairs_256_reverse` | 3.34 µs (quick) | Build a 256-property map from reverse-sorted pairs. PR-local quick A/B: 32.2 µs → 3.34 µs after the sort+dedup constructor rewrite. |
| `core_vector_value/construct_validate/128/768/1536` | 55.4 ns / 276 ns / 528 ns (quick) | Validate finite, non-empty `f32` vectors while constructing `VectorValue`; roughly linear in dimension. |
| `core_vector_value/clone_arc/128/768/1536` | 3.12 ns / 3.12 ns / 3.13 ns (quick) | Clone `VectorValue` shared component storage; intentionally dimension-independent. |
| `core_vector_value/postcard_roundtrip/128/768/1536` | 240 ns / 1.04 µs / 2.07 µs (quick) | Serialize and deserialize `Value::Vector`, including deserialize-time invariant checks. |
| `core_vector_distance/squared_euclidean/128/768/1536` | 18.8 ns / 113.9 ns / 249.1 ns (quick) | Exact lower-is-better L2-squared metric, safe `f64x4` accumulation; previous `f64x2` row was 22.3 ns / 203 ns / 447 ns. |
| `core_vector_distance/cosine/128/768/1536` | 32.0 ns / 179.9 ns / 358.2 ns (quick) | Exact cosine distance with zero-norm checks and clamped similarity; previous `f64x2` row was 36.5 ns / 255 ns / 506 ns. |
| `core_vector_distance/negative_inner_product/128/768/1536` | 15.7 ns / 114.0 ns / 240.6 ns (quick) | Max-inner-product adapter (`-dot`) with lower-is-better ordering; previous `f64x2` row was 21.1 ns / 194 ns / 421 ns. |
| `core_vector_exact_top_k/squared_euclidean_2048x128_k10` | ~42.0 µs (quick) | Exhaustive exact-search oracle over 2,048 candidates using a bounded max-heap (`O(n log k)`); previous `f64x2` row was 49.4 µs. |
| `core_vector_exact_top_k/cosine_2048x128_k10` | 54.8 µs (quick) | Bound-query cosine exact top-k over 2,048 candidates; the unbound comparison row is 65.2 µs. |

## §2 selene-graph — read hot paths

Bench bins: `single_graph`, `vector_index_rebuild`, `vector_pq`,
`vector_ivf_pq`, `vector_ivf_pressure`, `vector_mixed_workload`,
`bulk_mutation`, `concurrent_read`, `bfs`. The medians below predate CORE-06 (measured at the 128 B `Value`
layout); now that `Value` is 32 B, the `PropertyMap`-clone-heavy rows
(`graph_edge_create_cascade`, `graph_mutation_commit_batch`) will tighten at
the next full re-baseline. `graph_node_fetch` returns a column ref (no `Value`
clone) and is unaffected. `graph_exact_vector_scan/*` is the native graph-level
exact-vector oracle: label-filtered row scan plus the core vector metric
kernels, returning stable node ids. `graph_vector_index_rebuild/*` times the
maintenance rebuild that reclaims stale ANN entries after vector update/delete
churn; `graph_vector_index_recommended_rebuild/*` compares recommended-only
maintenance against full rebuild on a multi-index IVF fixture where only one
index is above the rebuild threshold. Fixture setup is excluded from the
reported Criterion duration.
`vector_pq` is a benchmark-only product-quantization candidate generator for
compression/recall research: PQ codes produce a short candidate set, then
full-fidelity vectors are exact reranked. `vector_ivf_pq` adds a coarse
synthetic IVF-style partition ahead of the same PQ scorer so future work can
compare standalone full-code scans against candidate-producer plus compression
layering. `vector_ivf_pressure` uses the production graph IVF index and records
list-skew plus candidate-pressure suffixes so future IVF/PQ layering work is
grounded against real index fanout under the expected 60% read / 40% write
workload. `vector_mixed_workload` also includes capped-maintenance cadence rows
that compare rebuilding one recommended IVF index per maintenance pass against
rebuilding every recommended IVF index after repeated 60/40 cycles. Vector
benchmark IDs include a memory/cardinality suffix:
`m{index KiB}-{reachable KiB}_n{indexed rows}_{flat|he...|ve...}`. The
`he...` form carries HNSW entries/live/deleted entries plus link counters; the
`ve...` form carries IVF entries/live/deleted entries plus centroid/list
counters. ANN recall IDs encode exact-ID recall as `idbp{basis points}` and
tie-tolerant nearest-distance quality as `dqbp{basis points}` before that
memory suffix.
unindexed rows use `noidx`. Rebuild IDs add
`upd{updates}_del{deletes}_b{entries-live-deleted}_a{entries-live-deleted}_rk{reclaimed reachable KiB}`.
Recommended-rebuild IDs add
`idx{registered indexes}_rb{rebuilt indexes}_pend{pending retrain entries}_bp{pending basis points}`.
Stale-query IDs use
`{stale|rebuilt}_n{rows}_{h|v}e{entries}l{live}d{deleted}_m{index KiB}-{reachable KiB}`,
where `h` is HNSW and `v` is IVF.
IVF pressure IDs use
`lists{centroids}ne{non_empty}max{max_list_len}avg{avg_list_len}avgq{avg_candidates_per_query}maxq{worst_case_candidates_per_query}_m{index KiB}-{reachable KiB}`.

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
| `graph_exact_vector_scan/squared_euclidean_dim128_k10` | 22.9 µs unindexed / 24.3 µs flat (quick) | Exhaustive label-filtered scan over 1,000 vector nodes; safe `f64x4` L2-squared accumulation; flat 20k row: ~244 µs. |
| `graph_exact_vector_scan/cosine_dim128_k10` | 33.5 µs unindexed / 33.6 µs flat (quick) | Exhaustive label-filtered scan over 1,000 vector nodes; safe `f64x4` cosine accumulation; flat 20k row: ~276 µs. |
| `graph_vector_index_rebuild/hnsw_l2_dim128_default` | 118.9 ms (quick) | Rebuilds a 128-dim HNSW L2 index after 10% vector updates + 5% deletes; compact level-0 links preserve the same link counts while reclaiming 150 stale HNSW entries. |
| `graph_vector_index_rebuild/hnsw_l2_dim128_m24ef64` | 200.7 ms (quick) | Tuned `M=24, ef_construction=64` rebuild row; keeps the high-recall research config covered with compacted post-rebuild level-0 links. |
| `graph_vector_index_rebuild/hnsw_cos_dim128_default` | 146.1 ms (quick) | Same rebuild fixture for 128-dim HNSW cosine, covering construction-side scorer reuse for metrics with bound query state. |
| `graph_vector_index_rebuild/hnsw_cos_dim128_m24ef64` | 247.1 ms (quick) | Tuned cosine rebuild row; link counts and recall shape are unchanged, but level-0 storage compacts after rebuild. |
| `graph_vector_index_rebuild/ivf_l2_dim128` | 2.108 ms (quick) | IVF rebuild row for the same 1k / 10% update / 5% delete fixture; replacements reuse IVF entries, so the suffix now reclaims only 50 delete-stale entries (`b1k-950-50`). |
| `graph_vector_index_rebuild/ivf_cos_dim128` | 2.124 ms (quick) | IVF cosine rebuild row with replacement reuse; bound cosine scorer cost is now mostly hidden by deterministic centroid retraining at this scale. |
| `graph_vector_index_recommended_rebuild/ivf_l2_dim128_recommended` | 2.256 ms at 1k / 12.92 ms at 10k (quick) | Multi-index IVF fixture with 4 registered indexes and one hot index above the rebuild threshold. Recommended maintenance rebuilds only the hot index (`idx4_rb1`). |
| `graph_vector_index_recommended_rebuild/ivf_l2_dim128_full` | 8.022 ms at 1k / 46.68 ms at 10k (quick) | Same fixture with full rebuild (`idx4_rb4`), grounding the avoided cold-index rebuild cost for maintenance orchestration. |
| `graph_vector_index_recommended_rebuild/ivf_cos_dim128_recommended` | 2.131 ms at 1k / 12.63 ms at 10k (quick) | Cosine variant of the recommended-only maintenance row. |
| `graph_vector_index_recommended_rebuild/ivf_cos_dim128_full` | 7.595 ms at 1k / 45.64 ms at 10k (quick) | Cosine full-rebuild comparison for the same 4-index fixture. |
| `graph_vector_index_stale_query/hnsw_l2_dim128_default` | 11.24 µs stale / 10.92 µs rebuilt (quick) | 1k fixture after 10% updates + 5% deletes. Stale overlay/mutable state reports `m478-1028`; rebuild compacts to `m212-687`. |
| `graph_vector_index_stale_query/hnsw_cos_dim128_default` | 12.12 µs stale / 12.59 µs rebuilt (quick) | Same churn shape under cosine. On this small fixture, rebuild is still a memory-control operation more than a strict query-latency win. |
| `graph_vector_index_stale_query/hnsw_l2_dim128_m24ef64` | 14.02 µs stale / 13.85 µs rebuilt (quick) | Tuned `M=24, ef_construction=64`; stale `m578-*` compacts to rebuilt `m258-*` while latency stays effectively neutral. |
| `graph_vector_index_stale_query/hnsw_cos_dim128_m24ef64` | 14.90 µs stale / 14.93 µs rebuilt (quick) | Tuned cosine row; memory compaction is visible, latency delta is noise-scale. |
| `graph_vector_index_stale_query/ivf_l2_dim128` | 19.30 µs stale / 19.69 µs rebuilt (quick) | IVF L2 probes 64 lists on the 1k fixture; replacement reuse plus delete-unlinking keeps assigned entries live-only while storage holes still compact from `ve1kl950d50_m46-562` to `ve950l950d0_m47-537`. |
| `graph_vector_index_stale_query/ivf_cos_dim128` | 18.97 µs stale / 19.43 µs rebuilt (quick) | IVF cosine query row; update churn no longer inflates stale candidate checks, and deleted entries are unlinked from probed lists before rebuild storage compaction. |
| `graph_vector_index_dimension_projection/hnsw_l2_default_dim128` | 10.98 µs (quick) | 1k HNSW L2 query row with suffix `m221-721`: ~221 KiB index-owned bytes and ~721 KiB reachable bytes after compact level-0 storage. |
| `graph_vector_index_dimension_projection/hnsw_l2_default_dim768` | 42.34 µs (quick) | Same HNSW topology/link count as dim128; reachable bytes rise to ~3.15 MiB because full-precision vector components dominate. |
| `graph_vector_index_dimension_projection/hnsw_l2_default_dim1536` | 81.01 µs (quick) | Reachable bytes rise to ~6.08 MiB at 1k vectors; extrapolation pressure is raw vector storage, not graph-link storage. |

PR-local PQ candidate compression spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_pq_candidate_recall/cluster_l2/m16_k16_c64_d128_k10_recallbp2250_m1570-full50000` | 8.75 ms (quick) | Benchmark-only product quantization over 100k 128-dim vectors and 16 queries. The compressed codebook+codes footprint is ~1.53 MiB vs ~48.8 MiB full vectors, but 64 candidates is too narrow for standalone recall. |
| `graph_pq_candidate_recall/cluster_l2/m16_k16_c256_d128_k10_recallbp9062_m1570-full50000` | 9.62 ms (quick) | Same compression footprint; widening exact rerank to 256 candidates restores most top-k overlap while staying under 10 ms for the 16-query batch. |
| `graph_pq_candidate_recall/cluster_l2/m16_k16_c1024_d128_k10_recallbp10000_m1570-full50000` | 12.60 ms (quick) | High-recall anchor: 1024 rerank candidates reaches 10000 bp on this corpus, but exact rerank cost becomes visible. |
| `graph_pq_candidate_recall/cluster_l2/m16_k64_c64_d128_k10_recallbp4625_m1594-full50000` | 8.87 ms (quick) | Larger subquantizer codebooks improve 64-candidate recall, with compressed storage still only ~1.56 MiB. |
| `graph_pq_candidate_recall/cluster_l2/m16_k64_c256_d128_k10_recallbp9500_m1594-full50000` | 9.68 ms (quick) | Best medium-width row: higher recall than `k16` at the same 256-candidate rerank width, for a small codebook-memory increase. |
| `graph_pq_candidate_recall/cluster_l2/m16_k64_c1024_d128_k10_recallbp10000_m1594-full50000` | 12.73 ms (quick) | Matches the 10000 bp high-recall row; useful as the baseline for future IVF/HNSW plus PQ layering rather than standalone full-code scans. |

PR-local IVF+PQ layering spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c256_p1_d128_k10_recallbp9500_rows25008_m2407-full50000` | 472.64 µs (quick) | Coarse synthetic IVF-style partition probes one list per query, then PQ scores and exact-reranks 256 candidates. Scans ~25k total rows across 16 queries and keeps the full-code PQ row's 9500 bp recall while using ~2.35 MiB compressed/coarse index memory. |
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c256_p2_d128_k10_recallbp9500_rows50010_m2407-full50000` | 609.20 µs (quick) | Probing two lists doubles candidate rows but does not improve recall on this corpus, which suggests the synthetic partition is already separating the query clusters cleanly. |
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c1024_p1_d128_k10_recallbp10000_rows25008_m2407-full50000` | 1.0475 ms (quick) | High-recall layered row: matches standalone PQ's 10000 bp result while scanning ~25k rows across the 16-query batch instead of 1.6M full-code rows, running roughly 12x faster than the standalone `m16_k64_c1024` row. |
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c1024_p2_d128_k10_recallbp10000_rows50010_m2407-full50000` | 1.1879 ms (quick) | Two-list probe keeps perfect recall but adds work without benefit on the clustered fixture; useful as a guardrail when future fixtures are less separable. |

PR-local production IVF candidate-pressure spot-check:

| Bench | 10k | Notes |
|---|---:|---|
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w1_idbp9750_dqbp9750_lists100ne100max137avg100avgq100maxq137...` | 93.19 µs (quick) | Production IVF index with 100 lists, all non-empty, max list 137, average list 100. Width 1 is close to HNSW latency but misses one oracle hit across the 16-query clustered-cosine fixture. |
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w2_idbp10000_dqbp10000_lists100ne100max137avg100avgq200maxq274...` | 140.96 µs (quick) | High-recall knee after the clean-index fast path: perfect recall at about 200 average candidates/query and 274 worst-case candidates/query, making this the production-IVF pressure baseline for 60/40 read/write planning. |
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w4_idbp10000_dqbp10000_lists100ne100max137avg100avgq400maxq548...` | 254.74 µs (quick) | Keeps perfect recall but doubles candidate pressure versus width 2; useful as the first guardrail for less separable future fixtures. |
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w64_idbp10000_dqbp10000_lists100ne100max137avg100avgq6400maxq8768...` | 2.9860 ms (quick) | Width 64 is excessive on this corpus: it scans about 64% of the corpus per query on average and mainly bounds the high-probe tail. |

After graph-level ANN row-hit conversion stopped re-heaping already bounded index
hits, the same quick run measured width 1 / 2 / 4 / 64 at 92.38 µs / 140.91 µs /
254.37 µs / 2.997 ms. The delta is intentionally treated as noise-scale at
`k=10`; the value is avoiding redundant heap work on the read path while
preserving dead-row filtering and deterministic `NodeId` tie ordering.

The same pressure bench also carries a `k=50` top-k sweep to catch wider
agent-memory retrieval requests. On the 10k clustered-cosine fixture, width 1
falls to 8725 bp recall/quality at ~112.4 µs, while width 2 restores 10000 bp at
~160.8 µs. Width 4 / 8 / 64 remain perfect-recall guardrails at ~274.4 µs /
~494.1 µs / ~3.065 ms.

PR-local IVF incremental-insert drift spot-check. Bench IDs include
`_d{basis_points}bp`; the suffix also includes pending-retrain pressure as
`pend{count}pdbp{basis_points}`.

| Drift | Mode | 1k width-2 | 1k recall/quality | 10k width-2 | 10k recall/quality | Notes |
|---:|---|---:|---:|---:|---:|---|
| 1% | incremental | 151.48 µs | 7063 bp | 108.06 µs | 3188 bp | Tiny novel clusters can be badly missed at default width before retraining. |
| 1% | rebuilt | 37.70 µs | 3500 bp | 92.06 µs | 10000 bp | At 1k, retraining too early hurts width-2 recall; at 10k it restores recall and lowers latency. |
| 5% | incremental | 161.46 µs | 8500 bp | 125.89 µs | 7313 bp | Drift becomes visible but still scale-sensitive. |
| 5% | rebuilt | 39.11 µs | 6000 bp | 90.99 µs | 10000 bp | 10k has enough novel mass for rebuild to be clearly useful. |
| 10% | incremental | 176.51 µs | 9563 bp | 150.12 µs | 8250 bp | Matches the original 10% drift signal: default width is degraded before retrain. |
| 10% | rebuilt | 38.23 µs | 10000 bp | 121.81 µs | 10000 bp | Rebuild restores default-width recall once the novel cluster is large enough. |

Width-64 guardrail rows still recover recall without rebuild, but they are much
slower than width-2 on the same fixture: 1k ranges from ~357-400 µs incremental
and ~342-370 µs rebuilt; 10k ranges from ~2.38-3.26 ms incremental and
~2.35-3.13 ms rebuilt. This supports a measured retrain policy over simply
raising IVF's default probe width under the 60% read / 40% write workload.

PR-local ANN recall spot-check:

| Bench | 10k | Notes |
|---|---:|---|
| `graph_ann_recall_validation/cluster_cos_hnsw_d128_k10_ef10_idbp9875_dqbp9875` | 108.25 µs (quick) | Default `M=18, ef_construction=64`; suffix starts `m2491-7491_n10k_he10k...`, so compact HNSW level-0 storage keeps index-owned memory around 2.43 MiB while ID-overlap and distance-quality recall stay at 9875 bp on this 10k corpus. |
| `graph_ann_recall_validation/cluster_cos_hnsw_m24ef64_d128_k10_ef10_idbp10000_dqbp10000` | 130.78 µs (quick) | Configured `M=24, ef_construction=64`; suffix starts `m2975-7975_n10k_he10k...`. Reaches 10000 bp ID-overlap and distance-quality recall, but costs ~21% slower ef10 search and ~19% more index memory than the default HNSW row. |
| `graph_ann_recall_validation/cluster_cos_ivf_d128_k10_ef10_idbp10000_dqbp10000` | 768.18 µs (quick) | First IVF recall row for the same 10k corpus; suffix starts `m615-5665_n10k_ve10k...c100q100a10k`, so IVF uses far less index-owned memory than HNSW and reaches 10000 bp recall/quality, but current probe/rerank defaults are much slower. |

PR-local IVF probe sweep:

| Bench | 10k | Notes |
|---|---:|---|
| `graph_ann_recall_validation/cluster_cos_ivf_d128_k10_ef1_idbp9750_dqbp9750` | 128.38 µs (quick) | One-list IVF probe is close to tuned-HNSW latency but misses one exact-oracle hit on the 16-query clustered-cosine fixture. |
| `graph_ann_recall_validation/cluster_cos_ivf_d128_k10_ef2_idbp10000_dqbp10000` | 194.98 µs (quick) | Two-list IVF probe restores 10000 bp ID-overlap and distance-quality recall while cutting latency ~3.9x versus ef10 and ~22x versus ef64 on this fixture. |
| `graph_ann_recall_validation/cluster_cos_ivf_d128_k10_ef4_idbp10000_dqbp10000` | 349.02 µs (quick) | Still perfect on this corpus, but extra probes are already dominated by exact rerank work. |

PR-local mixed vector read/write spot-check:

| Bench | 1k | 10k | Notes |
|---|---:|---:|---|
| `graph_vector_mixed_workload/ivf_cos_dim128_k10_r60w40_ef2` | 2.09 ms | 7.48 ms | One measured cycle interleaves 60 IVF cosine ANN reads and 40 vector-property updates over a 128-dim index. Fixture build is excluded; timed replacement writes reuse IVF entries, so routine updates no longer add stale IVF rows before rebuild compaction. |
| `graph_vector_mixed_workload/ivf_cos_dim128_k10_r60w40x10_ef2_maint_cap1` | 51.67 ms | 84.64 ms | Ten measured cycles run 60 reads / 40 writes per cycle across four IVF cosine indexes, then rebuild at most one recommended index. Each index reaches 100 pending retrain updates before maintenance. |
| `graph_vector_mixed_workload/ivf_cos_dim128_k10_r60w40x10_ef2_maint_all` | 57.23 ms | 116.28 ms | Same fixture and 10-cycle workload, but maintenance rebuilds every recommended IVF index. At 10k this isolates the cost of rebuilding four drifted indexes instead of pacing maintenance one index at a time. |

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
| `procedure_vector_search/shared_cache_squared_euclidean_dim128_k10_1000` | 37.0 µs (quick) | Cached `CALL selene.vector_search_nodes` over 1,000 vector nodes; scalar exact scan. |
| `procedure_vector_search/shared_cache_flat_index_dim128_k10_1000` | 37.5 µs (quick) | Cached exact search over the flat vector index. |
| `procedure_vector_search/shared_cache_hnsw_ann_dim128_k10_1000` | 13.46 µs (quick) | Cached single-query `CALL selene.vector_search_nodes_ann` over the HNSW index; graph-level ANN hit conversion no longer re-heaps index results. |
| `procedure_vector_search/shared_cache_hnsw_ann_repeated_8x_dim128_k10_1000` | 114.4 µs (quick) | Eight separate cached ANN procedure calls, one short-lived session per query. |
| `procedure_vector_search/shared_cache_hnsw_ann_batch_8x_dim128_k10_1000` | 108.9 µs (quick) | One cached `CALL selene.vector_search_nodes_ann_batch` over eight query vectors; ~4.5% below repeated single-call latency. |

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
