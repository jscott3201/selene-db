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
run-benches.sh --bench vector_graph_retrieval --compile-only  # compile tripwire, no Criterion run
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
tripwire. The `cosine_omlx_*` exact-top-k rows pin product-shaped local embedding
dimensions and candidate widths without depending on the localhost oMLX service.

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
| `core_vector_exact_top_k/cosine_omlx_{64/256/1024/4096}x1024_k10` | 12.2 µs / 47.6 µs / 188.3 µs / 750.7 µs (quick) | Product-shaped cosine rerank envelope for the 1024-dim local embedding model. |
| `core_vector_exact_top_k/cosine_omlx_{64/256/1024/4096}x2560_k10` | 29.6 µs / 116.6 µs / 465.1 µs / 1.856 ms (quick) | Product-shaped cosine rerank envelope for the 2560-dim local embedding model. |
| `core_vector_exact_top_k/cosine_omlx_{64/256/1024/4096}x4096_k10` | 47.0 µs / 185.5 µs / 739.3 µs / 2.959 ms (quick) | Product-shaped cosine rerank envelope for the 4096-dim local embedding model. |

## §2 selene-graph — read hot paths

Bench bins: `single_graph`, `vector_index_rebuild`, `vector_pq`,
`vector_ivf_pq`, `vector_ivf_pressure`, `vector_mixed_workload`,
`bulk_mutation`, `concurrent_read`, `bfs`, `text_search_bm25`. The medians below predate CORE-06 (measured at the 128 B `Value`
layout); now that `Value` is 32 B, the `PropertyMap`-clone-heavy rows
(`graph_edge_create_cascade`, `graph_mutation_commit_batch`) will tighten at
the next full re-baseline. `graph_node_fetch` returns a column ref (no `Value`
clone) and is unaffected. `graph_exact_vector_scan/*` is the native graph-level
exact-vector oracle: label-filtered row scan plus the core vector metric
kernels, returning stable node ids. `graph_vector_candidate_set/*` measures the
Rust graph/vector boundary for deriving canonical candidate sets from graph
adjacency before scoring. `graph_vector_index_rebuild/*` times the
maintenance rebuild that reclaims stale ANN entries after vector update/delete
churn; `graph_vector_index_recommended_rebuild/*` compares recommended-only
maintenance against full rebuild on a multi-index IVF fixture where only one
index is above the rebuild threshold. `graph_text_bm25_exact/*` is the
dependency-light full-text correctness oracle: it scans string properties,
computes query-local BM25 statistics, and returns deterministic top-k text hits.
`graph_text_bm25_indexed/*` compares a reusable in-memory postings index against
the oracle: `prebuilt_*` is the repeated-query path, while `transient_*` includes
index construction so build cost stays visible. Fixture setup is excluded from
the reported Criterion duration. `graph_json_contains_scan/*` is the exact JSON
metadata containment oracle over JSON-valued node properties before maintained
JSON/path indexes exist.
`graph_snapshot_read_loops/*` amortizes thread setup over many
`SharedGraph::read()` calls so the ArcSwap snapshot hot path is visible; the
older `graph_concurrent_reads` row remains a legacy spawn/join smoke row.
The focused `graph_vector_index_ivf_target_centroid_rebuild/*` group sweeps
explicit IVF list-count targets on the same rebuild fixture so read-side
candidate pressure can be compared against write-side retrain/reassignment cost.
`vector_pq` is a benchmark-only quantized candidate generator for
compression/recall research: PQ, dequantized scalar u8, scalar u8 code-space
distance, and packed binary sign codes produce short candidate sets, then
full-fidelity vectors are exact reranked.
`vector_ivf_pq` adds a coarse synthetic IVF-style partition ahead of PQ,
scalar code-space, and binary scorers so future work can compare standalone
full-code scans against candidate-producer plus compression layering.
`vector_ivf_pressure` uses the
production graph IVF index and records list-skew plus candidate-pressure
suffixes so future IVF/PQ layering work is grounded against real index fanout
under the expected 60% read / 40% write workload. It also includes the
`graph_ivf_target_centroids` sweep for explicit IVF list-count tuning.
`vector_mixed_workload` includes capped-maintenance cadence rows that compare
rebuilding one recommended IVF index per maintenance pass against rebuilding
every recommended IVF index after repeated 60/40 cycles. Vector benchmark IDs
include a memory/cardinality suffix:
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
| `graph_label_index_lookup` | 7.83 ns | 7.88 ns | 8.09 ns | Flat; `DbString`-keyed hash lookup. |
| `graph_typed_index_point` | 15.25 ns | 15.05 ns | 15.26 ns | Flat tri-state `lookup_eq`. |
| `graph_typed_index_range` | 7.05 µs | 37.89 µs | 55.74 µs | Sub-linear range scan. |
| `graph_composite_index_proxy` | 82.8 ns | 177.1 ns | 313.9 ns | Linear. |
| `graph_edge_create_cascade` | 362.9 µs | 747.4 µs | 1.481 ms | Mutation + commit body; teardown excluded. |
| `graph_mutation_commit_batch` (10) | 336.7 µs | 307.6 µs | 446.9 µs | Batched commit, 10 ops. |
| `graph_mutation_commit_batch` (100) | 408.2 µs | 420.8 µs | 552.7 µs | Batched commit, 100 ops. |
| `graph_mutation_commit_batch` (1000) | 952.4 µs | 1.053 ms | 1.226 ms | Batched commit, 1000 ops. |
| `graph_concurrent_reads` | 74.6 µs | 71.7 µs | 71.8 µs | Legacy row: 10 scoped threads with one snapshot read each; dominated by spawn/join. |
| `graph_snapshot_read_loops/single_thread` | 334.14 µs | 336.52 µs | 337.36 µs | 100k snapshot reads per sample, about 3.34-3.37 ns/read; scale-flat. |
| `graph_snapshot_read_loops/parallel_threads8` | 15.508 ms | 11.209 ms | 10.955 ms | 8 threads x 20k reads per sample, about 69-97 ns/read including scoped thread setup and contention. |
| `graph_bfs` (depth=1) | 106.3 ns | 109.0 ns | 109.6 ns | Depth-1 independent of N. |
| `graph_bfs` (depth=10) | 11.34 µs | 12.09 µs | 12.18 µs | Mostly traversal cost. |
| `graph_bfs` (depth=50) | 101.1 µs | 111.1 µs | 113.1 µs | Saturates ~110 µs. |

PR-local quick text baseline:

| Bench | 1k | Notes |
|---|---:|---|
| `graph_text_bm25_exact/topic_query/n1000_k10` | 327.59 µs (quick) | Exact BM25 scan over 1,000 string-valued document nodes with Unicode-aware tokenization, query-local document frequencies, and deterministic score/node-id ordering. This is the oracle for postings-index and hybrid BM25/vector rows. |
| `graph_text_bm25_indexed/prebuilt_topic_query/n1000_k10` | 34.665 µs (quick) | Repeated query over a prebuilt `TextIndex` postings structure. Same BM25 tokenizer/scorer/order as the exact oracle; about 9.5x faster than the exact scan on this fixture. |
| `graph_text_bm25_indexed/transient_build_query/n1000_k10` | 456.56 µs (quick) | Build a transient postings index from the graph snapshot, then query it once. Slower than exact for one-off 1k queries; useful as the build-cost envelope and as the bridge toward durable maintained registrations. |

PR-local quick JSON baseline:

| Bench | 1k | Notes |
|---|---:|---|
| `graph_json_contains_scan/nested_metadata_k10/1000` | 21.731 µs (quick) | Exact scan over 1,000 JSON metadata payloads with one-quarter matching nested current episodic facts, skipping non-JSON properties. This is the oracle for future maintained JSON/path indexes and JSON/vector/text candidate composition. |

PR-local quick vector baseline:

| Bench | 1k | Notes |
|---|---:|---|
| `graph_exact_vector_scan/squared_euclidean_dim128_k10` | 22.9 µs unindexed / 24.3 µs flat (quick) | Exhaustive label-filtered scan over 1,000 vector nodes; safe `f64x4` L2-squared accumulation; flat 20k row: ~244 µs. |
| `graph_exact_vector_scan/cosine_dim128_k10` | 33.5 µs unindexed / 33.6 µs flat (quick) | Exhaustive label-filtered scan over 1,000 vector nodes; safe `f64x4` cosine accumulation; flat 20k row: ~276 µs. |
| `graph_vector_candidate_set/neighbor_candidates_depends_on_k64` | 233.8 ns (quick) | Derives a sorted/deduplicated 64-node candidate set from one anchor's outgoing `DEPENDS_ON` adjacency. This measures the reusable Rust candidate-set boundary, not vector scoring. |
| `graph_vector_candidate_set/adjacency_label_range_l8_k64` | 44.6 ns (quick) | Iterates the sorted label range for 64 matching edges mixed with 8x64 unrelated-label edges. |
| `graph_vector_candidate_set/adjacency_label_scan_l8_k64` | 374.8 ns (quick) | Benchmark-local old path: scans the same mixed-label adjacency entry and filters by label, showing the range lookup is ~8.4x faster for high-degree mixed-label candidates. |
| `graph_vector_candidate_state/maintained_active_c512_total1024` | 343.9 ns (quick) | Materializes a provider-maintained 512-node current set from a 1,024-node fixture with stale nodes disqualified by `SUPERSEDED_BY`. |
| `graph_vector_candidate_state/dynamic_active_scan_c512_total1024` | 12.79 µs (quick) | Benchmark-local query-time baseline: scans all 1,024 document nodes and checks outgoing `SUPERSEDED_BY`, showing maintained state is ~37x faster for this currentness slice. |
| `graph_vector_candidate_set/set_intersection_l256_r256_o128` | 153.2 ns (quick) | Intersects two canonical 256-node sets with 128 overlapping ids using the merge path; this is the balanced graph/ANN/active-set composition primitive. |
| `graph_vector_candidate_set/set_intersection_l8_r1024_o8` | 31.10 ns (quick) | Intersects a tiny dependency-style set with a much larger maintained active set using the binary-search probe path. |
| `graph_vector_candidate_set/set_union_l256_r256_o128` | 170.6 ns (quick) | Unions two canonical 256-node sets into a 384-node canonical candidate set. |
| `graph_vector_candidate_set/set_difference_l256_r256_o128` | 178.5 ns (quick) | Computes the graph-side exclusion path for two canonical 256-node sets with 128 overlapping ids. |
| `graph_vector_candidate_set/from_search_hits_l256_r256_o128` | 179.8 ns (quick) | Builds a canonical candidate set from 256 vector-search hits, covering ANN/search-output composition. |
| `graph_vector_index_rebuild/hnsw_l2_dim128_default` | 118.9 ms (quick) | Rebuilds a 128-dim HNSW L2 index after 10% vector updates + 5% deletes; compact level-0 links preserve the same link counts while reclaiming 150 stale HNSW entries. |
| `graph_vector_index_rebuild/hnsw_l2_dim128_m24ef64` | 200.7 ms (quick) | Tuned `M=24, ef_construction=64` rebuild row; keeps the high-recall research config covered with compacted post-rebuild level-0 links. |
| `graph_vector_index_rebuild/hnsw_cos_dim128_default` | 146.1 ms (quick) | Same rebuild fixture for 128-dim HNSW cosine, covering construction-side scorer reuse for metrics with bound query state. |
| `graph_vector_index_rebuild/hnsw_cos_dim128_m24ef64` | 247.1 ms (quick) | Tuned cosine rebuild row; link counts and recall shape are unchanged, but level-0 storage compacts after rebuild. |
| `graph_vector_index_rebuild/ivf_l2_dim128` | 2.108 ms (quick) | IVF rebuild row for the same 1k / 10% update / 5% delete fixture; replacements reuse IVF entries, so the suffix now reclaims only 50 delete-stale entries (`b1k-950-50`). |
| `graph_vector_index_rebuild/ivf_cos_dim128` | 2.124 ms (quick) | IVF cosine rebuild row with replacement reuse; bound cosine scorer cost is now mostly hidden by deterministic centroid retraining at this scale. |
| `graph_vector_index_ivf_target_centroid_rebuild/ivf_cos_dim128_default` | 1.704 ms at 1k / 11.25 ms at 10k (quick) | Default IVF target-list rebuild baseline for the 10% update / 5% delete fixture; suffixes reclaim 50 stale entries at 1k and 500 at 10k. |
| `graph_vector_index_ivf_target_centroid_rebuild/ivf_cos_dim128_c16` | 950.5 µs at 1k / 3.267 ms at 10k (quick) | Coarse 16-list rebuild row; cheapest retrain/reassignment cost, but read-side target-centroid pressure showed coarse lists can hurt 10k candidate fanout. |
| `graph_vector_index_ivf_target_centroid_rebuild/ivf_cos_dim128_c128` | 6.625 ms at 1k / 13.08 ms at 10k (quick) | Explicit 128-list rebuild row; near default cost at 10k, but much more expensive than coarse lists at 1k. |
| `graph_vector_index_ivf_target_centroid_rebuild/ivf_cos_dim128_c512` | 26.23 ms at 1k / 46.46 ms at 10k (quick) | Over-wide 512-list rebuild row; matches the read-side finding that very high list counts add cost before improving this fixture. |
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

PR-local scalar quantization spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_scalar_quant_candidate_recall/cluster_l2/u8_c64_d128_k10_recallbp10000_m12501-full50000` | 79.90 ms (quick) | Benchmark-only per-dimension u8 scalar quantization over 100k 128-dim vectors and 16 queries. Compressed storage is ~12.2 MiB vs ~48.8 MiB full vectors, and 64 exact-rerank candidates reach 10000 bp recall on this clustered fixture. |
| `graph_scalar_quant_candidate_recall/cluster_l2/u8_c256_d128_k10_recallbp10000_m12501-full50000` | 81.00 ms (quick) | Wider rerank has no recall upside on this corpus and adds a small exact-rerank cost. The full compressed scan remains the dominant cost. |
| `graph_scalar_quant_candidate_recall/cluster_l2/u8_c1024_d128_k10_recallbp10000_m12501-full50000` | 85.35 ms (quick) | High-candidate anchor for comparison with PQ. Scalar quantization is simple and training-free, but standalone row-wise dequantized scoring is much slower than PQ and IVF+PQ candidate generation without SIMD/block scoring. |
| `graph_scalar_code_quant_candidate_recall/cluster_l2/u8code_c64_d128_k10_recallbp10000_m12501-full50000` | 20.49 ms (quick) | Ranks by integer L2 over per-dimension u8 codes, then exact-reranks full vectors. It keeps scalar u8's full recall and storage shape while avoiding row-wise dequantization, but remains slower than packed binary and standalone PQ full-recall rows. |
| `graph_scalar_code_quant_candidate_recall/cluster_l2/u8code_c256_d128_k10_recallbp10000_m12501-full50000` | 21.41 ms (quick) | Wider rerank has no recall upside and adds modest exact-rerank cost; c64 is the scalar code-space knee on this fixture. |
| `graph_scalar_code_quant_candidate_recall/cluster_l2/u8code_c1024_d128_k10_recallbp10000_m12501-full50000` | 24.96 ms (quick) | High-candidate scalar code-space anchor. This narrows scalar's cost gap versus dequantized scoring by roughly 4x, but still does not beat simpler packed binary sign-code filtering. |

PR-local binary quantization spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_binary_quant_candidate_recall/cluster_l2/sign_c32_d128_k10_recallbp3375_m1562-full50000` | 3.35 ms (quick) | Benchmark-only packed sign-bit quantization over 100k 128-dim vectors and 16 queries. Memory is ~1.53 MiB vs ~48.8 MiB full vectors, but 32 candidates is too narrow. |
| `graph_binary_quant_candidate_recall/cluster_l2/sign_c64_d128_k10_recallbp6625_m1562-full50000` | 3.46 ms (quick) | Hamming prefilter plus exact rerank remains fast, but 64 candidates still loses material recall on this clustered L2 fixture. |
| `graph_binary_quant_candidate_recall/cluster_l2/sign_c256_d128_k10_recallbp10000_m1562-full50000` | 4.10 ms (quick) | First full-recall binary row: same compressed footprint as narrow binary rows, roughly 3x faster than standalone PQ full-recall rows and much smaller/faster than scalar u8. |
| `graph_binary_quant_candidate_recall/cluster_l2/sign_c1024_d128_k10_recallbp10000_m1562-full50000` | 7.15 ms (quick) | Wider exact rerank has no recall upside on this fixture and doubles latency versus the c256 knee. |

PR-local IVF+PQ layering spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c256_p1_d128_k10_recallbp9500_rows25008_m2407-full50000` | 472.64 µs (quick) | Coarse synthetic IVF-style partition probes one list per query, then PQ scores and exact-reranks 256 candidates. Scans ~25k total rows across 16 queries and keeps the full-code PQ row's 9500 bp recall while using ~2.35 MiB compressed/coarse index memory. |
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c256_p2_d128_k10_recallbp9500_rows50010_m2407-full50000` | 609.20 µs (quick) | Probing two lists doubles candidate rows but does not improve recall on this corpus, which suggests the synthetic partition is already separating the query clusters cleanly. |
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c1024_p1_d128_k10_recallbp10000_rows25008_m2407-full50000` | 1.0475 ms (quick) | High-recall layered row: matches standalone PQ's 10000 bp result while scanning ~25k rows across the 16-query batch instead of 1.6M full-code rows, running roughly 12x faster than the standalone `m16_k64_c1024` row. |
| `graph_ivf_pq_candidate_recall/cluster_l2/m16_k64_c1024_p2_d128_k10_recallbp10000_rows50010_m2407-full50000` | 1.1879 ms (quick) | Two-list probe keeps perfect recall but adds work without benefit on the clustered fixture; useful as a guardrail when future fixtures are less separable. |

PR-local IVF+scalar-code layering spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_ivf_scalar_code_candidate_recall/cluster_l2/u8code_c64_p1_d128_k10_recallbp10000_rows25008_m13314-full50000` | 526.58 µs (quick) | Synthetic IVF probes one list per query, then ranks probed rows by integer L2 over per-dimension u8 codes before exact rerank. It reaches full recall at the narrow c64 width and is about 2x faster than IVF+PQ full-recall p1, but uses ~13.0 MiB compressed/coarse memory. |
| `graph_ivf_scalar_code_candidate_recall/cluster_l2/u8code_c256_p1_d128_k10_recallbp10000_rows25008_m13314-full50000` | 819.44 µs (quick) | Wider rerank has no quality upside and adds exact-rerank cost; c64 is the scalar-code IVF knee on this fixture. |
| `graph_ivf_scalar_code_candidate_recall/cluster_l2/u8code_c256_p2_d128_k10_recallbp10000_rows50010_m13314-full50000` | 1.1598 ms (quick) | Two-list probing doubles searched rows with no recall upside on this separable corpus. |
| `graph_ivf_scalar_code_candidate_recall/cluster_l2/u8code_c1024_p1_d128_k10_recallbp10000_rows25008_m13314-full50000` | 1.3434 ms (quick) | High-candidate scalar-code IVF anchor. It is slower than the narrow c64 row and slower than IVF+PQ full-recall p1, so code-space scalar needs a memory/quality reason before it beats simpler alternatives. |

PR-local IVF+binary layering spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_ivf_binary_candidate_recall/cluster_l2/sign_c64_p1_d128_k10_recallbp6625_rows25008_m2375-full50000` | 150.2 µs (quick) | Synthetic IVF probes one list per query, then Hamming-scores packed sign bits and exact-reranks 64 candidates. Very fast, but still too narrow for recall. |
| `graph_ivf_binary_candidate_recall/cluster_l2/sign_c256_p1_d128_k10_recallbp10000_rows25008_m2375-full50000` | 309.8 µs (quick) | High-recall binary layered row: scans the same ~25k rows as IVF+PQ p1 but reaches 10000 bp about 3.4x faster than the `m16_k64_c1024_p1` IVF+PQ row, with similar compressed/coarse memory. |
| `graph_ivf_binary_candidate_recall/cluster_l2/sign_c256_p2_d128_k10_recallbp10000_rows50010_m2375-full50000` | 363.3 µs (quick) | Two-list probe doubles searched rows but has no recall upside on this separable fixture; p1 is the better knee. |
| `graph_ivf_binary_candidate_recall/cluster_l2/sign_c1024_p1_d128_k10_recallbp10000_rows25008_m2375-full50000` | 901.6 µs (quick) | Wider exact rerank stays below IVF+PQ full-recall latency but has no quality benefit over c256 on this corpus. |

PR-local IVF overlap-corpus compression spot-check:

| Bench | 100k | Notes |
|---|---:|---|
| `graph_ivf_overlap_candidate_recall/pq_overlap_l2/m16_k64_c1024_p1_d128_k10_recallbp5000_rows24996_m2407-full50000` | 1.334 ms (quick) | Harder overlap profile where cluster signal competes with local variation. One-list probing misses half the oracle hits even with 1024 PQ rerank candidates. |
| `graph_ivf_overlap_candidate_recall/pq_overlap_l2/m16_k64_c1024_p4_d128_k10_recallbp10000_rows100005_m2407-full50000` | 2.407 ms (quick) | Four-list probing restores full recall for the IVF+PQ high-recall row, but searches about 100k rows across the 16-query batch. |
| `graph_ivf_overlap_candidate_recall/pq_overlap_l2/m16_k64_c4096_p4_d128_k10_recallbp10000_rows100005_m2407-full50000` | 7.766 ms (quick) | Wider PQ rerank has no quality upside over 1024 candidates after four-list probing and mostly measures exact-rerank cost. |
| `graph_ivf_overlap_candidate_recall/binary_overlap_l2/sign_c256_p1_d128_k10_recallbp5000_rows24996_m2375-full50000` | 411.1 µs (quick) | Binary Hamming keeps the same recall failure mode as PQ under one-list probing, so the candidate producer, not the compressed scorer width, is the limiting factor here. |
| `graph_ivf_overlap_candidate_recall/binary_overlap_l2/sign_c256_p4_d128_k10_recallbp10000_rows100005_m2375-full50000` | 711.7 µs (quick) | Four-list probing restores full recall and remains about 3.4x faster than the IVF+PQ full-recall p4 row at similar compressed/coarse memory. |
| `graph_ivf_overlap_candidate_recall/binary_overlap_l2/sign_c1024_p1_d128_k10_recallbp5000_rows24996_m2375-full50000` | 1.013 ms (quick) | Wider exact rerank cannot recover missing coarse lists; recall remains 5000 bp. |
| `graph_ivf_overlap_candidate_recall/binary_overlap_l2/sign_c1024_p4_d128_k10_recallbp10000_rows100005_m2375-full50000` | 1.821 ms (quick) | Wider binary rerank has no recall upside over c256 after p4 and is slower, confirming c256 remains the overlap-profile knee. |

PR-local production IVF candidate-pressure spot-check:

| Bench | 10k | Notes |
|---|---:|---|
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w1_idbp9750_dqbp9750_lists100ne100max137avg100avgq100maxq137...` | 93.19 µs (quick) | Production IVF index with 100 lists, all non-empty, max list 137, average list 100. Width 1 is close to HNSW latency but misses one oracle hit across the 16-query clustered-cosine fixture. |
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w2_idbp10000_dqbp10000_lists100ne100max137avg100avgq200maxq274...` | 140.96 µs (quick) | High-recall knee after the clean-index fast path: perfect recall at about 200 average candidates/query and 274 worst-case candidates/query, making this the production-IVF pressure baseline for 60/40 read/write planning. |
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w4_idbp10000_dqbp10000_lists100ne100max137avg100avgq400maxq548...` | 254.74 µs (quick) | Keeps perfect recall but doubles candidate pressure versus width 2; useful as the first guardrail for less separable future fixtures. |
| `graph_ivf_candidate_pressure/cluster_cos/d128_k10_w64_idbp10000_dqbp10000_lists100ne100max137avg100avgq6400maxq8768...` | 2.9860 ms (quick) | Width 64 is excessive on this corpus: it scans about 64% of the corpus per query on average and mainly bounds the high-probe tail. |

PR-local explicit IVF target-centroid sweep:

| Variant | 1k perfect-recall knee | 10k perfect-recall knee | Notes |
|---|---:|---:|---|
| `ivf_default` | width 2 / 94.80 µs | width 2 / 139.67 µs | Default `ceil(sqrt(n))` list count: 32 lists at 1k, 100 lists at 10k. Width 2 reaches 10000 bp recall/quality at both scales and remains the current 10k knee. |
| `ivf_c16` | width 2 / 82.25 µs | width 1 / 621.23 µs | Low list count keeps recall perfect but creates very large 10k lists (`avg625`, max ~2k), so read cost scales poorly despite low centroid-scoring overhead. |
| `ivf_c128` | width 4 / 66.46 µs | width 2 / 137.30 µs | At 1k, 128 lists with width 4 is the fastest perfect-recall row; at 10k, width 2 is slightly faster than default while using more centroid-scoring work. |
| `ivf_c512` | no perfect row at width <= 4 | width 2 / 230.78 µs | High list count reduces candidate rows but pays heavy centroid scoring. On 1k it only reaches 7750 bp at width 4; on 10k width 2 is perfect but slower than default/c128 width 2. |

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
| `graph_vector_mixed_workload/point_read_ivf_update_r60w40_dim128` | 1.625 ms | 6.978 ms | Same IVF vector fixture and 40 indexed vector-property updates, but the 60 reads are point `node_properties` lookups. This isolates routine IVF update maintenance from ANN query cost. |
| `graph_vector_mixed_workload/ivf_cos_dim128_k10_r60w40x10_ef2_maint_cap1` | 51.67 ms | 84.64 ms | Ten measured cycles run 60 reads / 40 writes per cycle across four IVF cosine indexes, then rebuild at most one recommended index. Each index reaches 100 pending retrain updates before maintenance. |
| `graph_vector_mixed_workload/ivf_cos_dim128_k10_r60w40x10_ef2_maint_all` | 57.23 ms | 116.28 ms | Same fixture and 10-cycle workload, but maintenance rebuilds every recommended IVF index. At 10k this isolates the cost of rebuilding four drifted indexes instead of pacing maintenance one index at a time. |

## §3 selene-graph — write pipeline & concurrency

Bench bins: `write_txn_lifecycle`, `provider_fanout`, `bound_type_validation`,
`concurrent_writers`, `graph_hub_delete`, `graph_read_under_write`,
`graph_mixed_workload`.

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
| `provider_fanout/active_set_edge_create_k40` | 40 edge creates + active-set provider | 283.2 µs | In-memory commit/provider path for `CONTRADICTS`-style active-set removal; no WAL. |
| `provider_fanout/active_set_edge_delete_k40` | 40 edge deletes + active-set provider | 218.8 µs | Delete path uses provider-owned `edge_id -> source` state to reinsert active nodes; seed excluded from timed body. |
| `provider_fanout/active_set_wal_edge_create_k40` | 40 edge creates + WAL + active-set provider | 4.75 ms | Core WAL durability plus provider removal; provider state itself remains in-memory. |
| `provider_fanout/active_set_wal_edge_delete_k40` | 40 edge deletes + WAL + active-set provider | 4.21 ms | Core WAL durability plus provider reinsertion; seed excluded from timed body. |
| `provider_fanout/active_hint_recent_edge_create_k40` | 40 `RECENT_IN` creates + active-hint provider | 242.5 µs | Maintains window→member state in provider memory; no WAL. |
| `provider_fanout/active_hint_recent_edge_delete_k40` | 40 `RECENT_IN` deletes + active-hint provider | 199.4 µs | Delete path uses provider-owned edge provenance to remove window members. |
| `provider_fanout/active_hint_wal_recent_edge_create_k40` | 40 `RECENT_IN` creates + WAL + active-hint provider | 4.78 ms | Core WAL durability dominates active-hint membership maintenance. |
| `provider_fanout/active_hint_wal_recent_edge_delete_k40` | 40 `RECENT_IN` deletes + WAL + active-hint provider | 4.41 ms | WAL-backed delete path remains near the active-set WAL boundary. |
| `provider_fanout/active_hint_dependency_edge_create_k40` | 40 `DEPENDS_ON` creates + active-hint provider | 300.3 µs | Maintains anchor→dependency state for one broad task anchor; no WAL. |
| `provider_fanout/active_hint_dependency_edge_delete_k40` | 40 `DEPENDS_ON` deletes + active-hint provider | 199.4 µs | Delete path removes dependency targets through provider-owned edge provenance. |
| `provider_fanout/active_hint_wal_dependency_edge_create_k40` | 40 `DEPENDS_ON` creates + WAL + active-hint provider | 4.57 ms | Core WAL durability dominates dependency maintenance. |
| `provider_fanout/active_hint_wal_dependency_edge_delete_k40` | 40 `DEPENDS_ON` deletes + WAL + active-hint provider | 4.55 ms | WAL-backed dependency deletes stay in the same cost band as active-set deletes. |
| `bound_type_validation/unbound_commit` | 10k / 50k / 100k | 291 / 246 / 320 µs | Commit without graph-type validation. |
| `bound_type_validation/bound_commit_simple` | 10k / 50k / 100k | 304 / 250 / 350 µs | Typed-commit validation delta (small). |
| `bound_type_validation/bound_commit_rich` | 10k / 50k / 100k | 1.01 / 1.14 / 1.67 ms | Wider type-graph validation delta. |
| `bound_type_validation/bound_schema_change` | 10k / 50k / 100k | 2.92 / 18.6 / 39.3 ms | Full graph-state revalidation; scales with N. |
| `graph_mixed_workload/point_read_update_r60w40` | 10k / 50k / 100k | 9.207 / 11.045 / 16.699 ms | One scalar cycle: 60 snapshot point reads interleaved with 40 non-indexed property-update commits. Fixture clone/setup excluded; no vector index or WAL. |
| `graph_mixed_workload/point_read_indexed_update_r60w40` | 10k / 50k / 100k | 9.261 / 11.129 / 16.842 ms | Same scalar cycle, but the 40 writes update `Person.age`, a registered typed property index. The close delta to the non-indexed row keeps property-index maintenance below the dominant sequential commit cost at these scales. |
| `graph_mixed_workload/candidate_state_edge_update_r60w40` | 10k / 50k / 100k | 3.196 / 5.310 / 11.826 ms | One maintained candidate-state cycle: 60 generation-checked `current` set reads plus 20 `SUPERSEDED_BY` edge deletes and 20 creates. Exercises provider reactivation and invalidation without WAL. |
| `graph_mixed_workload/candidate_state_metadata_edge_update_r60w40` | 10k / 50k / 100k | 2.976 / 4.333 / 9.922 ms | Same provider write cycle, but the 60 reads fetch generation-checked candidate-state metadata rather than materializing the full `current` set. The widening delta against the full-set row isolates set materialization cost. |
| `graph_mixed_workload/point_read_update_r60w40_wal` | 10k / 50k / 100k | 139.11 / 130.07 / 134.45 ms | Same scalar 60/40 cycle backed by a real per-iteration WAL tempdir with committer batching off. Setup/teardown excluded; the near scale-flat cost shows per-commit durability barriers dominate this sequential 40-write shape. |

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

#### `persist_wal_payload_shape_*` — scalar / JSON / vector payloads

These rows keep the WAL format unchanged and isolate payload shape for the
future WAL/compression overhaul. Quick profile writes/replays 1k changes as ten
100-change entries with `SyncPolicy::OnFlushOnly`; setup is outside the replay
timed body. The JSON fixture models an agent-memory metadata document, and the
vector fixtures use 128-dim and 768-dim first-class `Value::Vector` payloads.

Command:

```bash
scripts/run-benches.sh --profile quick --bench wal --filter payload_shape
```

| Bench | scalar i64 | JSON metadata | vector128 | vector768 | Notes |
|---|---:|---:|---:|---:|---|
| `persist_wal_payload_shape_no_fsync` | 1.084 ms | 1.677 ms | 1.826 ms | 2.185 ms | Append path only; no fsync in timed body. |
| `persist_wal_payload_shape_replay` | 1.433 ms | 2.870 ms | 2.514 ms | 2.908 ms | Reader open + checksum + optional decompression + postcard decode. |

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
| `gql_expression_eval/*` (16 cases) | 180–245 ns, plus JSON rows below | Scalar eval: predicates, scalar fns, CASE, list access, binary ops, and runtime-parameter JSON scalar functions. |
| `procedure_call_repeat/no_cache` | 2.958 ms | 100 short-lived sessions, parse/analyze/plan each. |
| `procedure_call_repeat/shared_cache` | 27.49 µs | Shared `Arc<CallPlanCache>` warm-hit — **99.1% lower**. |

PR-local quick JSON expression baseline. These rows bind `Value::Json` payloads
as runtime parameters, so the timed body measures expression execution and
JSON scalar work, not JSON text parsing during `json(...)`.

| Bench | Median | Notes |
|---|---:|---|
| `gql_expression_eval/json/parse_type` | 151.83 ns (quick) | `json_type($payload)` over a prebound agent-memory metadata document. |
| `gql_expression_eval/json/nested_get_path_text` | 215.31 ns (quick) | Nested object/array path selector returning an episodic fact title. |
| `gql_expression_eval/json/has_path_miss` | 210.55 ns (quick) | Same nested selector shape returning a deterministic missing-path boolean. |
| `gql_expression_eval/json/contains_nested` | 189.99 ns (quick) | Recursive containment against a prebound candidate object/array subset. |
| `gql_expression_eval/json/construct_metadata` | 596.09 ns (quick) | Builds a nested JSON metadata document from scalar runtime parameters. |
| `gql_expression_eval/json/merge_patch_metadata` | 351.87 ns (quick) | Applies an RFC 7396 merge patch to a prebound metadata document. |
| `gql_expression_eval/json/patch_metadata` | 472.89 ns (quick) | Applies a three-operation RFC 6902 JSON Patch to a prebound metadata document. |

PR-local quick vector procedure baseline:

| Bench | Median | Notes |
|---|---:|---|
| `procedure_vector_search/shared_cache_squared_euclidean_dim128_k10_1000` | 37.0 µs (quick) | Cached `CALL selene.vector_search_nodes` over 1,000 vector nodes; scalar exact scan. |
| `procedure_vector_search/shared_cache_flat_index_dim128_k10_1000` | 25.0 µs (quick) | Cached exact search over the flat vector index. |
| `procedure_vector_search/shared_cache_flat_index_repeated_8x_dim128_k10_1000` | 199.0 µs (quick) | Eight separate cached exact procedure calls, one short-lived session per query. |
| `procedure_vector_search/shared_cache_flat_index_batch_8x_dim128_k10_1000` | 176.3 µs (quick) | One cached `CALL selene.vector_search_nodes_batch` over eight query vectors; ~11% below repeated exact single-call latency. |
| `procedure_vector_search/shared_cache_score_nodes_64_dim128_k10_1000` | 5.15 µs (quick) | Cached `CALL selene.vector_score_nodes` over a 64-node candidate set; graph-derived candidate rerank baseline. |
| `procedure_vector_search/shared_cache_score_nodes_repeated_8x64_dim128_k10_1000` | 45.4 µs (quick) | Eight separate cached candidate-score procedure calls, one short-lived session per query. |
| `procedure_vector_search/shared_cache_score_nodes_batch_8x64_dim128_k10_1000` | 43.0 µs (quick) | One cached `CALL selene.vector_score_nodes_batch` over eight query vectors and eight 64-node candidate sets; ~5% below repeated single-call latency. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_64_dim128_k10_1000` | 3.97 µs (quick) | Cached `CALL selene.vector_score_candidate_state` over one maintained 64-node candidate set; avoids caller-side node-list construction and validates the generation-checked provider path. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_repeated_8x64_dim128_k10_1000` | 32.75 µs (quick) | Eight separate cached maintained candidate-state score calls, one short-lived session per query; faster than explicit-node repeated scoring. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_nodes_intersection_64_dim128_k10_1000` | 4.86 µs (quick) | Cached `CALL selene.vector_score_candidate_state_nodes` intersecting a maintained 64-node state with a 64-node explicit candidate list before exact rerank. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_nodes_intersection_repeated_8x64_dim128_k10_1000` | 39.65 µs (quick) | Eight separate cached maintained-state + explicit-node intersection calls, one short-lived session per query. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_nodes_union_128_dim128_k10_1000` | 8.20 µs (quick) | Cached maintained-state + explicit-node union producing 128 canonical candidates before exact rerank. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_nodes_state_difference_32_dim128_k10_1000` | 3.04 µs (quick) | Cached maintained-state minus explicit-node candidates, leaving 32 candidates before exact rerank. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_expanded_intersection_64_dim128_k10_1000` | 4.88 µs (quick) | Cached `CALL selene.vector_score_candidate_state_expanded` expanding two graph roots through `SUPPORTS`, intersecting with maintained state, and exact-reranking 64 canonical candidates. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_expanded_intersection_repeated_8x64_dim128_k10_1000` | 39.69 µs (quick) | Eight separate cached maintained-state + graph-expanded intersection calls, one short-lived session per query. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_expanded_intersection_batch_8x64_dim128_k10_1000` | 36.83 µs (quick) | One cached `CALL selene.vector_score_candidate_state_expanded_batch` over eight query/root-set pairs; composes maintained state with graph-expanded roots in one procedure call, ~7% below repeated expanded-state latency. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_expanded_union_128_dim128_k10_1000` | 8.25 µs (quick) | Cached maintained-state + graph-expanded union producing 128 canonical candidates before exact rerank. |
| `procedure_vector_candidate_state/shared_cache_score_candidate_state_expanded_state_difference_32_dim128_k10_1000` | 3.13 µs (quick) | Cached maintained-state minus graph-expanded candidates, leaving 32 candidates before exact rerank. |
| `procedure_vector_neighbors/shared_cache_score_neighbors_64_dim128_k10_1000` | 4.60 µs (quick) | Cached `CALL selene.vector_score_neighbors` over one 64-neighbor graph-derived candidate set. |
| `procedure_vector_neighbors/shared_cache_score_neighbors_repeated_8x64_dim128_k10_1000` | 40.3 µs (quick) | Eight separate cached graph-neighbor score calls, one short-lived session per query. |
| `procedure_vector_neighbors/shared_cache_score_neighbors_batch_8x64_dim128_k10_1000` | 36.7 µs (quick) | One cached `CALL selene.vector_score_neighbors_batch` over eight anchors; ~9% below repeated neighbor-call latency. |
| `procedure_vector_expanded/shared_cache_score_expanded_2root64_dim128_k10_1000` | 4.85 µs (quick) | Cached `CALL selene.vector_score_expanded_candidates` where two root nodes expand through `SUPPORTS` to a 64-node canonical candidate set. |
| `procedure_vector_expanded/shared_cache_score_expanded_repeated_8x2root64_dim128_k10_1000` | 42.81 µs (quick) | Eight separate cached expanded-candidate score calls, one short-lived session per query. |
| `procedure_vector_expanded/shared_cache_score_expanded_batch_8x2root64_dim128_k10_1000` | 41.31 µs (quick) | One cached `CALL selene.vector_score_expanded_candidates_batch` over eight per-query root sets; ~3.5% below repeated expanded-call latency. |
| `procedure_vector_expanded/shared_cache_score_expanded_query_roots_2root64_dim128_k10_1000` | 95.94 µs (quick) | Full GQL pipeline row: `MATCH` + `WITH collect_list(root)` derives two graph roots, then calls `selene.vector_score_expanded_candidates`; root production dominates the explicit-root procedure boundary. |
| `procedure_vector_search/shared_cache_hnsw_ann_dim128_k10_1000` | 13.46 µs (quick) | Cached single-query `CALL selene.vector_search_nodes_ann` over the HNSW index; graph-level ANN hit conversion no longer re-heaps index results. |
| `procedure_vector_search/shared_cache_hnsw_ann_repeated_8x_dim128_k10_1000` | 114.4 µs (quick) | Eight separate cached ANN procedure calls, one short-lived session per query. |
| `procedure_vector_search/shared_cache_hnsw_ann_batch_8x_dim128_k10_1000` | 108.9 µs (quick) | One cached `CALL selene.vector_search_nodes_ann_batch` over eight query vectors; ~4.5% below repeated single-call latency. |
| `procedure_vector_ann_expanded/shared_cache_ann_expanded_2root64_dim128_k10_1000` | 14.83 µs (quick) | Cached `CALL selene.vector_search_expanded_candidates_ann`; HNSW supplies two `VectorSummary` roots, graph expansion walks `SUPPORTS`, and exact rerank returns final `VectorFact` candidates. |
| `procedure_vector_ann_expanded/shared_cache_ann_expanded_repeated_8x2root64_dim128_k10_1000` | 123.41 µs (quick) | Eight separate cached ANN-root graph-expansion calls, one short-lived session per query. |
| `procedure_vector_ann_expanded/shared_cache_ann_expanded_batch_8x2root64_dim128_k10_1000` | 119.18 µs (quick) | One cached `CALL selene.vector_search_expanded_candidates_ann_batch` over eight query vectors; ~3.4% below repeated ANN-expanded-call latency. |
| `procedure_vector_ann_expanded/shared_cache_ann_state_expanded_intersection_2root64_dim128_k10_1000` | 15.20 µs (quick) | Cached `CALL selene.vector_search_candidate_state_expanded_ann` using HNSW roots, graph expansion, maintained-state intersection, and exact rerank. |
| `procedure_vector_ann_expanded/shared_cache_ann_state_expanded_intersection_repeated_8x2root64_dim128_k10_1000` | 127.98 µs (quick) | Eight separate cached ANN-root graph-expansion calls intersected with maintained state before exact rerank. |

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
| `write_e2e/gql_cached_point_read_set_r60w40` | 8.717 ms | 6.781 ms | 9.318 ms | One warm-plan-cache in-memory cycle: 60 indexed `bench_id` point reads plus 40 indexed `SET score = $score` writes over two parameterized source strings. |
| `write_e2e/gql_multi_statement_txn_preplanned` | 280 µs | 191 µs | 350 µs | START, three INSERTs, COMMIT. |
| `write_e2e/explicit_txn_3_inserts_rust_api` | 275 µs | 223 µs | 363 µs | Three inserts via the Rust txn API. |
| `write_e2e/explicit_txn_3_inserts_rollback` | 279 µs | 198 µs | 355 µs | Same, rolled back. |
| `write_e2e/gql_insert_single_node_preplanned_with_flush` | 4.22 ms | 4.27 ms | 3.95 ms | Durable: preplanned insert + WAL flush. |
| `write_e2e/direct_insert_single_node_with_wal_flush` | 4.20 ms | 4.30 ms | 4.17 ms | Direct mutation + one WAL flush. |
| `write_e2e/direct_insert_single_node_with_wal_flush_every10` | 30.5 ms | 32.2 ms | 32.4 ms | Ten direct inserts over one flush. |

PR-local quick JSON mixed row:

| Bench | Median | Notes |
|---|---:|---|
| `write_e2e/gql_cached_json_read_patch_r60w40/1000` | 4.486 ms (quick) | One warm-plan-cache in-memory cycle over property-backed JSON payloads: 60 indexed `bench_id` point reads extracting nested JSON metadata and 40 indexed point updates applying an idempotent three-op `json_patch` over `payload`. Payload seeding runs outside the timed body. |

## §6 selene-algorithms

Bench bins: `algo_bench`, `projection`, `vector_graph_retrieval`. Fixture:
`BenchFixture::build(N)` (≈3N edges) for pagerank/betweenness/apsp and
projection; `planted_community_graph(N)` (≈6N edges, ~N/64 communities) for
triangle_count and louvain. `vector_graph_retrieval` is the first native
graph+vector agent-memory research fixture: it stores topic-summary vectors
plus support, temporal-validity, and supersession edges to evidence nodes, then
compares vector-only ANN against PageRank rerank, graph expansion,
validity-aware expansion, supersession-aware expansion, and an exact
graph/vector oracle. Row IDs encode total graph coverage as
`covbp{basis points}`, current-valid coverage as `curbp{basis points}`, and
topic precision as `precbp{basis points}`.

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

### §6c Graph-Augmented Vector Retrieval Research

Quick rows below are local research fixtures, not production API claims. The
fixture intentionally makes top-k vector retrieval semantically redundant:
nearest summaries are high-precision but low-coverage, while one-hop `SUPPORTS`
expansion can recover evidence facts from the graph. Half of evidence nodes are
stale by construction; `VALID_AT` edges identify current evidence, while
`SUPERSEDED_BY` edges link stale evidence to a current replacement. This lets
the fixture separate raw coverage from current-valid coverage, then compare
filtering against graph repair. PageRank scores and WCC component candidates are
computed in fixture setup through real `GraphProjection` paths; timed rows
measure retrieval only. Expanded graph candidates are exact-scored through the
native candidate scoring primitive before fact-diverse selection. The exact
graph oracle uses exact vector search plus validity-aware graph expansion to
bound achievable fixture quality. The companion `graph_vector_component_pressure`
group widens graph-derived component pools before the same exact candidate
scorer, exposing when graph-bounded scoring stops being cheap enough to beat
ANN or compressed pre-scoring.

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_retrieval/vector_only/...covbp2459_curbp2459_precbp9677` | 206.8 µs | 502.5 µs (`covbp1328_curbp1308_precbp9687`) | Baseline ANN top-k: high topic precision, poor evidence coverage because summaries dominate nearest neighbors. |
| `graph_vector_retrieval/pagerank_prior/...covbp2459_curbp2459_precbp9677` | 224.9 µs | 523.7 µs (`covbp1289_curbp1289_precbp9687`) | Negative result: PageRank reranking alone does not add missing evidence candidates and can slightly hurt coverage at 10k. |
| `graph_vector_retrieval/graph_expand/...covbp9435_curbp4838_precbp9516` | 311.5 µs | 711.4 µs (`covbp8203_curbp4335_precbp8828`) | Raw one-hop expansion improves total coverage but often selects stale evidence; native candidate scoring trims rerank overhead. |
| `graph_vector_retrieval/graph_expand_valid/...covbp9395_curbp9395_precbp9395` | 291.7 µs | 665.5 µs (`covbp7929_curbp7929_precbp8183`) | Validity-aware expansion prunes stale candidates, making current-valid coverage match total coverage while running faster than raw expansion. |
| `graph_vector_retrieval/graph_expand_superseded/...covbp9395_curbp9395_precbp9395` | 282.7 µs | 728.2 µs (`covbp8203_curbp8203_precbp8339`) | Supersession-aware expansion repairs stale candidates through `SUPERSEDED_BY`; native candidate scoring makes this cheaper than validity filtering at 1k and narrows the 10k cost. |
| `graph_vector_retrieval/graph_expand_valid_wide/...covbp9556_curbp9556_precbp9677` | 319.5 µs | 1.160 ms (`covbp8281_curbp8281_precbp9687`) | Wider 16-hit ANN seeding improves current-valid coverage and precision, but the larger candidate fanout costs ~75% more than narrow validity-aware expansion at 10k. |
| `graph_vector_retrieval/graph_expand_superseded_wide/...covbp9556_curbp9556_precbp9677` | 310.8 µs | 1.162 ms (`covbp8281_curbp8281_precbp9687`) | Wide supersession matches wide validity quality on this fixture; supersession repair has no extra coverage upside once the wider seed already reaches current evidence. |
| `graph_vector_retrieval/graph_component_filter/...covbp10000_curbp10000_precbp10000` | 50.5 µs | 109.8 µs (`covbp10000_curbp10000_precbp10000`) | WCC-derived component filtering exact-scores only the query anchor's graph component, reaching oracle quality while avoiding global exact scan and ANN fanout. This is the first strong positive graph-acceleration row. |
| `graph_vector_retrieval/graph_expand_pagerank/...covbp9435_curbp4838_precbp9516` | 358.0 µs | 794.8 µs (`covbp8203_curbp4335_precbp8828`) | PageRank on top of raw expansion adds cost without current-valid uplift on this fixture; keep as a guardrail before promoting algorithm-prior policies. |
| `graph_vector_retrieval/exact_graph_oracle/...covbp10000_curbp10000_precbp10000` | 1.414 ms | 22.2 ms | Exact vector search plus validity-aware expansion reaches the fixture oracle, but it is far slower at 10k and only suitable as a research bound. |

Local-only embedding rows are disabled unless `SELENE_EMBEDDING_BENCH=1` (or
the legacy `SELENE_OMLX_EMBEDDING_BENCH=1`) is set. They call the developer's
local oMLX OpenAI-compatible endpoint by default, or OpenRouter when
`SELENE_EMBEDDING_PROVIDER=openrouter` is set, and therefore are **not**
expected to run in CI. Use ignored `.env` keys only through the shell
environment; do not commit or print them:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_CORPUS=tiny \
SELENE_EMBEDDING_BATCH_SIZE=64 \
SELENE_EMBEDDING_MODELS=Qwen3-Embedding-0.6B-4bit-DWQ,Qwen3-Embedding-4B-4bit-DWQ \
scripts/run-benches.sh --profile quick --bench vector_graph_retrieval --filter graph_vector_omlx_embedding_pressure --vector-scales 1000
```

OpenRouter Codestral Embed rows use the same corpora and benchmark surfaces:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=code_alias_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat --filter query_root_current_state_intersection_batch
```

The reverse-order Codestral text/vector row compares vector-expanded candidates
followed by BM25 over those explicit candidates:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=code_alias_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_vector_text_batch|query_root_current_state_text_score_batch|query_root_current_state_intersection_batch|query_root_expansion_batch'
```

In that comparison run, the companion medians were 146.35 us for
current-state BM25, 222.94 us for current-state vector scoring, and 224.65 us
for plain graph-expanded vector scoring.

The wider 16-query code/alias profile uses the same benchmark row family:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=code_alias_wide_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The project-code profile uses curated source-shaped snippets with real
selene-db module and symbol names:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=project_code_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The project-code alias profile keeps real module/symbol names but adds
natural-language decoys around the same concepts:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=project_code_alias_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The project source-code profile uses short real selene-db source excerpts as
target documents, plus source-shaped distractors and currentness decoys:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=project_source_code_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The project source-chunk profile uses target-aware implementation snippets from
current selene-db modules. The first two documents per topic are non-target
graph-root hints, so `SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2` keeps targets in the
expanded support set:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=project_source_chunk_memory \
SELENE_EMBEDDING_BATCH_SIZE=4 \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The project source-file profile embeds selected real selene-db files as
file-level documents. Use a smaller setup batch size for external providers so
full-file inputs are sent in conservative chunks:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=project_source_file_memory \
SELENE_EMBEDDING_BATCH_SIZE=4 \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The project migration profile uses archived/prototype terminology as query
aliases, current first-class engine surfaces as target documents, and stale
decoy documents that maintained current-state should exclude:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_PROVIDER=openrouter \
SELENE_EMBEDDING_CORPUS=project_migration_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat \
  --filter 'query_root_current_state_text_score_batch|query_root_current_state_text_vector_batch|query_root_current_state_intersection_batch|query_root_provenance_state_intersection_batch|query_root_expansion_batch|query_root_vector_text_batch'
```

The GQL query-root rows use the same local oMLX corpus but exercise the full
query pipeline. The materialization rows isolate `MATCH` plus
`WITH collect_list(root)` root production before any vector procedure runs; the
shape-pressure rows split that further into anchor lookup, root-row traversal,
and root-list aggregation. The reused-session rows compare root-only and
full-scoring shapes against a long-lived session with and without the existing
source-string `PlanCache`: plan caching collapses repeated root-only query
production, while full scoring is effectively neutral because vector rerank is
the dominant work. The scoring row derives `OmlxDependsOn` graph-hint roots,
passes them into `selene.vector_score_expanded_candidates`, and the procedure
expands roots through `OmlxSupports` before exact scoring. Its maintained-state
companion uses the same GQL-produced roots with
`selene.vector_score_candidate_state_expanded`, intersecting expanded roots
with the provider-maintained `omlx_support_facts` set. The negative-evidence
state row uses the same procedure over `omlx_current_support_facts`, where
documents containing stale/superseded/contradictory wording have an outgoing
`OmlxNegativeEvidence` edge and are excluded from current support state. The
provenance-required row uses `omlx_provenance_current_support_facts`, adding a
required incoming `OmlxSupports` edge and required outgoing `OmlxGroundedBy`
edge before the same exact rerank. The text-score rows use the same
GQL-produced roots, expand them through `OmlxSupports` in GQL, then call
`selene.text_score_nodes` over explicit candidate nodes using a maintained BM25
text index. The batched rows store each query vector and query text on the query
anchor and let GQL aggregate per-query root/candidate sets. The text batch row
calls `selene.text_score_nodes_batch` once for the full 16-query profile after
GQL expands graph roots to explicit candidates. The current-state text batch row
calls `selene.text_score_candidate_state_expanded_batch` once for the same
profile, keeping maintained-state composition, graph expansion, and BM25
scoring inside one procedure boundary. The text/vector fusion row uses the same
current-state BM25 batch row as a candidate producer, regroups its hits, and
then calls `selene.vector_score_nodes_batch` for exact vector rerank. The pure
expansion row calls `selene.vector_score_expanded_candidates_batch` once for the
full 16-query profile; the current/provenance vector rows call
`selene.vector_score_candidate_state_expanded_batch` so maintained state,
graph-expanded roots, and exact rerank stay inside one procedure boundary:

```bash
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_CORPUS=scaled_ambiguous_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat --filter procedure_vector_omlx_query_roots
```

Embedding requests are chunked by `SELENE_EMBEDDING_BATCH_SIZE` (or legacy
`SELENE_OMLX_EMBEDDING_BATCH_SIZE`; default: 64). Profiles above that size
preserve input order across multiple POSTs and fail if any response chunk does
not return exactly one vector per input.
`SELENE_GRAPH_HINT_DOCS_PER_TOPIC=N` caps graph-authored topic labels and
`OmlxDependsOn` edges to the first `N` same-topic documents per topic; unset
means every same-topic document receives graph hints. The partial-hint fixture
also adds `OmlxSupports` edges from graph-hint documents to same-topic support
facts so rows can compare direct partial hints, graph-expanded hints, and ANN
union against the same endpoint embeddings.
The fixture also registers a maintained candidate-state provider named
`omlx_support_facts` over an explicit `OmlxSupportFact` label, plus
`omlx_current_support_facts` over the same label with outgoing
`OmlxNegativeEvidence` edges as exclusions, and
`omlx_provenance_current_support_facts` over the same current-state rules plus
required incoming support and outgoing provenance edges. With partial graph
hints, graph-hint roots are not support facts; with uncapped hints, every
document remains a support fact. That models provider-maintained support facts
separately from graph-hint root documents without making the default uncapped
profile degenerate to an empty state. `curbp{basis points}` records current
support precision, while `basecurbp` records the direct graph-expanded row's
current precision before negative-evidence filtering. Target-aware local
profiles also add `hitbp{basis points}`, which records how many queries had
their expected target document in the returned top-k set.

The first local corpus is intentionally tiny (16 documents + 4 queries across
GQL, vector-index, agent-memory, and Rust-code topics). It validates that real
endpoint embeddings round-trip through `Value::Vector`, graph HNSW indexing,
exact cosine search, ANN search, and graph-label candidate-set scoring before
larger local corpus work. `SELENE_OMLX_CORPUS=agent_memory` expands that to
32 documents + 8 queries; `SELENE_OMLX_CORPUS=ambiguous_memory` keeps the same
shape but deliberately overlaps vocabulary across topics to stress vector-only
retrieval. `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` combines both 40-input
profiles into 64 documents + 16 queries, crossing the default batch size as a
64+16 request pair. `SELENE_OMLX_CORPUS=code_alias_memory` adds a smaller
target-aware code/alias profile; `SELENE_OMLX_CORPUS=code_alias_wide_memory`
extends that shape to 16 target queries.
`SELENE_OMLX_CORPUS=project_code_memory`,
`project_code_alias_memory`, `project_source_code_memory`,
`project_source_chunk_memory`, and `project_source_file_memory` use current
selene-db module names, implementation snippets, or selected full files to
stress code/source retrieval. `project_workspace_source_memory` reads the
current checkout at benchmark setup time and extracts small line-numbered
windows around named source symbols, so it can feed live project code into
local embedding rows without committing more stale snippets.
`project_migration_memory` uses stale archived/prototype wording as query and
decoy pressure while targeting current engine files and procedures. These
target-aware profiles keep topic/current precision metrics but also record
`hitbp` so rows can show whether the expected symbol/fact/file was retrieved,
not only whether the result was in the right broad topic:

| oMLX row | Qwen3 0.6B / 1024 dim | Qwen3 4B / 2560 dim | Notes |
|---|---:|---:|---|
| `graph_vector_omlx_embedding_pressure/embed_batch/...docs20_batch64` | 39.23 ms | 208.8 ms | End-to-end localhost embedding request for 20 texts. |
| `graph_vector_omlx_embedding_pressure/exact_graph_search/...precbp6875` | 13.58 µs | 31.81 µs | Exact cosine over 16 stored endpoint vectors and 4 query vectors. |
| `graph_vector_omlx_embedding_pressure/hnsw_graph_search/...precbp6875` | 16.07 µs | 34.29 µs | HNSW cosine over the same vectors (`k=4`, `ef=64`). |
| `graph_vector_omlx_embedding_pressure/topic_label_candidate_score/...c4...precbp10000` | 4.11 µs | 9.46 µs | Candidate sets are derived from graph topic labels and batch-scored exactly. |
| `graph_vector_omlx_embedding_pressure/topic_neighbor_score/...c4...precbp10000` | 4.04 µs | 9.41 µs | Query-anchor `OmlxDependsOn` edges derive same-topic candidates through one-hop graph-neighbor scoring. |
| `graph_vector_omlx_embedding_pressure/topic_neighbor_batch_score/...c4...precbp10000` | 4.07 µs | 9.46 µs | Batched one-hop neighbor scoring over the same tiny profile. |
| `SELENE_OMLX_CORPUS=agent_memory` `topic_label_candidate_score/...c8...precbp10000` | 15.88 µs | 34.96 µs | Expanded 32-document / 8-query agent-memory profile; graph labels still restore full precision. |
| `SELENE_OMLX_CORPUS=agent_memory` `topic_neighbor_score/...c8...precbp10000` | 15.51 µs | 34.80 µs | Explicit graph-neighbor candidate derivation stays full-precision on the expanded profile. |
| `SELENE_OMLX_CORPUS=agent_memory` `topic_neighbor_batch_score/...c8...precbp10000` | 15.61 µs | 34.74 µs | Batched one-hop neighbor scoring over the expanded profile. |
| `SELENE_OMLX_CORPUS=ambiguous_memory` `exact_graph_search/...` | 51.72 µs | 122.08 µs | Vector-only exact scan drops to `precbp6250` / `precbp4375` under cross-topic vocabulary overlap. |
| `SELENE_OMLX_CORPUS=ambiguous_memory` `hnsw_graph_search/...ef64...` | 74.46 µs | 152.15 µs | HNSW mirrors exact precision on this profile, but is slower at this tiny scale. |
| `SELENE_OMLX_CORPUS=ambiguous_memory` `topic_label_candidate_score/...c8...precbp10000` | 15.63 µs | 35.11 µs | Graph-label candidate sets restore full precision despite semantic cross-talk. |
| `SELENE_OMLX_CORPUS=ambiguous_memory` `topic_neighbor_score/...c8...precbp10000` | 15.47 µs | 34.93 µs | Explicit graph-neighbor candidates restore full precision with similar latency. |
| `SELENE_OMLX_CORPUS=ambiguous_memory` `topic_neighbor_batch_score/...c8...precbp10000` | 15.57 µs | 35.05 µs | Batched one-hop neighbor scoring over the ambiguity-stress profile. |
| `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` `embed_batch/...docs80_batch64` | 132.97 ms | 822.68 ms | Two local embedding POSTs (64 + 16 inputs) over the scaled 80-input profile. |
| `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` `exact_graph_search/...` | 199.12 µs | 478.63 µs | Vector-only exact scan drops to `precbp5625` / `precbp4843` at 64 documents + 16 queries. |
| `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` `hnsw_graph_search/...ef64...` | 295.10 µs | 583.72 µs | HNSW mirrors exact precision on the scaled profile, remaining slower at this local size. |
| `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` `topic_label_candidate_score/...c16...precbp10000` | 59.93 µs | 133.58 µs | Graph-label candidate sets restore full precision over 16 same-topic documents per query. |
| `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` `topic_neighbor_score/...c16...precbp10000` | 59.69 µs | 133.92 µs | Explicit graph-neighbor candidates preserve full precision with equivalent candidate width. |
| `SELENE_OMLX_CORPUS=scaled_ambiguous_memory` `topic_neighbor_batch_score/...c16...precbp10000` | 59.90 µs | 134.19 µs | Batched one-hop neighbor scoring over the scaled 80-input profile. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_label_candidate_score/...c2...precbp5000` | 9.65 µs | 22.46 µs | Partial graph labels cap graph-only precision at 2 of 4 hits per query while staying cheap. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_neighbor_score/...c2...precbp5000` | 9.63 µs | 22.31 µs | Partial graph-neighbor hints have the same c2 quality/cost shape as labels. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_neighbor_batch_score/...c2...precbp5000` | 9.64 µs | 22.40 µs | Batched partial neighbor hints are neutral at this tiny candidate width. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_label_ann_union_score/...precbp5625/4843...ann8` | 332.30 µs | 668.43 µs | Small ANN union raises the 1024-dim model only to vector-only precision and lowers the 2560-dim model below c2 graph-only precision. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_neighbor_ann_union_score/...precbp5625/4843...ann8` | 331.61 µs | 668.26 µs | Same negative fallback result through explicit graph-neighbor candidates. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_hint_expansion_score/...c16...precbp10000` | 61.74 µs | 135.29 µs | Two direct graph hints per topic expand through `OmlxSupports` to the full same-topic support set, restoring full precision with the same width as complete graph labels. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_hint_expansion_cached_score/...c16...precbp10000` | 58.32 µs | 132.14 µs | Precomputing the same expanded support candidate sets trims query-time graph traversal overhead while preserving full precision. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_hint_expansion_state_score/...c14...precbp10000` | 55.15 µs | 119.96 µs | Intersects graph-expanded hints with maintained `omlx_support_facts`, filtering root hint docs while preserving full precision. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_hint_expansion_refresh_sets/...q16_c16_totalc256` | 2.97 µs | 3.05 µs | Recomputes every cached support candidate set from graph topology and asserts it matches cached state; refresh cost is small at this hot-scope size. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_hint_expansion_cached_r60w40/...r60w40_totalc256` | 3.64 ms | 8.05 ms | Conservative mixed cycle with 60 cached candidate-set scoring reads plus 40 full graph-topology refreshes via the production candidate-expansion API; vector rerank dominates refresh work on this local profile. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `topic_hint_expansion_ann_union_score/...precbp5625/4843...ann8` | 371.82 µs (`c22`) | 754.71 µs (`c21`) | ANN union after full graph expansion hurts precision and adds hundreds of microseconds; avoid widening precise graph-expanded candidate sets with ANN by default. |
| `SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2` `ann_hint_expansion_state_score/...ann8` | 392.51 µs (`precbp5312`, `c44`) | 699.14 µs (`precbp4531`, `c42`) | ANN roots expanded through support edges and intersected with maintained support-fact state still miss too many target facts; avoid adding a batched ANN/state procedure until a workload shows better quality. |
| `procedure_vector_omlx_query_roots/shared_cache_query_anchor_lookup/...q16_anchors16...` | 703.69 µs | 691.54 µs | Repeated per-query lookup of one `OmlxQueryAnchor` by `query_index`, creating a fresh `Session` for each query. |
| `procedure_vector_omlx_query_roots/shared_session_query_anchor_lookup/...q16_anchors16...` | 697.91 µs | 684.55 µs | Reuses one session but does not enable `PlanCache`; this is effectively neutral versus fresh sessions. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_anchor_lookup/...q16_anchors16...` | 38.46 µs | 38.95 µs | Reuses one hot source-string `PlanCache` session; parse/analyze/plan are skipped for repeated parameterized root-shape queries. |
| `procedure_vector_omlx_query_roots/shared_cache_query_anchor_lookup_batch/...q16_anchors16...` | 54.73 µs | 54.58 µs | Single GQL statement returns all 16 query anchors ordered by `query_index`; this is the lower bound for batched root-shape execution. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_rows/...q16_r2_totalr32...` | 789.15 µs | 786.71 µs | Repeated per-query `OmlxDependsOn` traversal returns two root rows per query without `collect_list` aggregation. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_rows_batch/...q16_r2_totalr32...` | 67.73 µs | 67.55 µs | Single statement returns all 32 root rows; edge traversal adds ~13 µs over batched anchor lookup on this fixture. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_materialize/...q16_r2_totalr32...` | 918.45 µs | 918.35 µs | Repeated per-query GQL root production: `MATCH` + `collect_list(root)` materializes two roots for each of 16 query anchors, without vector procedure dispatch or scoring. |
| `procedure_vector_omlx_query_roots/shared_session_query_root_materialize/...q16_r2_totalr32...` | 912.40 µs | 917.28 µs | Reuses one session for the same materialization source, again showing little benefit without `PlanCache`. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_root_materialize/...q16_r2_totalr32...` | 52.96 µs | 53.70 µs | Reuses one hot `PlanCache` session for the materialization source, making repeated root production competitive with the batched root-row shape. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_materialize_batch/...q16_r2_totalr32...` | 78.85 µs | 77.95 µs | Single GQL statement materializes all 16 root sets; aggregation adds ~11 µs over batched root-row traversal before vector scoring enters the path. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_expansion/...q16_k4_r2_c16...precbp10000` | 1.45 ms | 1.52 ms | Full GQL row over the scaled partial-hint corpus: `MATCH` + `collect_list(root)` derives two roots per query, graph expansion restores the 16-document same-topic set, and all 64 returned top-k hits are on-topic; current-fact precision is lower (`basecurbp8593/8281`) because stale same-topic facts remain eligible. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_root_expansion/...q16_k4_r2_c16...precbp10000` | 1.45 ms | 1.52 ms | Same full scoring statement through a warmed source-string `PlanCache` session; plan caching is neutral once graph expansion and vector rerank dominate. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_text_score/...q16_k4_r2_c16...precbp10000_curbp9218` | 2.16 ms | 2.22 ms | Full GQL row over the same graph roots and support expansion, but reranks expanded candidates with maintained BM25 via `selene.text_score_nodes`; topic precision is full, while current-fact precision is partial because BM25 still admits some stale same-topic facts. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_root_text_score/...q16_k4_r2_c16...precbp10000_curbp9218` | 2.19 ms | 2.23 ms | Warmed source-string `PlanCache` session for the same text-score row; plan caching is effectively neutral because repeated GQL candidate production dominates. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_text_score_batch/...q16_k4_r2_c16...precbp10000_curbp9218` | 472.70 µs | 474.70 µs | Single GQL statement builds all 16 query texts and graph-expanded candidate sets, then calls `selene.text_score_nodes_batch` once; preserves full topic precision while exposing the same currentness gap as the repeated text scorer. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c13...precbp10000_curbp10000` | 180.19 µs | 182.71 µs | Single GQL statement builds all 16 query texts and root sets, then calls `selene.text_score_candidate_state_expanded_batch`; maintained current-state composition restores full current precision and avoids explicit candidate materialization. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c13...precbp10000_curbp10000` | 2.3434 ms | 5.3866 ms | Current-state BM25 batch produces top-k candidates, then `selene.vector_score_nodes_batch` reranks them. Quality stays full, but the extra vector pass is much slower on this fixture; do not recommend text/vector fusion here without a quality gap. |
| `SELENE_OMLX_CORPUS=code_alias_memory` `shared_cache_query_root_current_state_text_score_batch/...q8_k4_r2_c6...precbp5000_curbp5000_hitbp8750` | 144.57 µs | 145.39 µs | Target-aware code/alias profile. Sparse BM25 emits fewer than `k=4` rows per query, but seven of eight expected target facts appear in top-k; broad topic/current precision alone would miss this target-level signal. |
| `SELENE_OMLX_CORPUS=code_alias_memory` `shared_cache_query_root_current_state_text_vector_batch/...q8_k4_r2_c6...precbp5000_curbp5000_hitbp8750` | 553.96 µs | 1.0504 ms | Exact vector rerank after the same BM25/state candidate producer does not improve target hits on this profile and adds dimension-sensitive cost. |
| `SELENE_OMLX_CORPUS=code_alias_memory` `shared_cache_query_root_expansion_batch/...q8_k4_r2_c9...precbp10000_hitbp8750` | 178.74 µs | 257.01 µs | Plain graph-expanded vector scoring is faster than maintained-state vector scoring, but it still misses one expected target and does not apply the current-state gate. |
| `SELENE_OMLX_CORPUS=code_alias_memory` `shared_cache_query_root_current_state_intersection_batch/...q8_k4_r2_c6...basecurbp8125_curbp10000_hitbp10000` | 210.17 µs | 287.61 µs | Batched graph-root vector scoring through maintained current state recovers all expected code/alias targets. This is slower than BM25/state but fixes the missing-target case without adding a fusion API. |
| `SELENE_EMBEDDING_PROVIDER=openrouter` `mistralai/codestral-embed-2505` `shared_cache_query_root_expansion_batch/...q8_k4_r2_c9...dim1536_precbp10000_hitbp8750` | 211.05 µs | - | OpenRouter Codestral Embed 2505 through the same code-alias corpus. Plain graph-expanded vector scoring again misses one expected target. |
| `SELENE_EMBEDDING_PROVIDER=openrouter` `mistralai/codestral-embed-2505` `shared_cache_query_root_current_state_intersection_batch/...q8_k4_r2_c6...dim1536_basecurbp7812_curbp10000_hitbp10000` | 211.95 µs | - | Maintained current-state vector scoring recovers all expected code-alias targets with effectively the same latency as plain expansion on this profile. |
| `SELENE_EMBEDDING_PROVIDER=openrouter` `mistralai/codestral-embed-2505` `shared_cache_query_root_provenance_state_intersection_batch/...q8_k4_r2_c6...dim1536_basecurbp7812_curbp10000_hitbp10000` | 212.38 µs | - | Provenance-gated current-state vector scoring preserves the same target quality as the current-state vector batch with negligible extra latency on this code-alias fixture. |
| `SELENE_EMBEDDING_PROVIDER=openrouter` `mistralai/codestral-embed-2505` `shared_cache_query_root_current_state_text_score_batch/...q8_k4_r2_c6...dim1536_precbp5000_curbp5000_hitbp8750` | 136.79 µs | - | Maintained current-state BM25 remains the fastest Codestral-backed code-alias row, but sparse lexical matches still miss one expected target. |
| `SELENE_EMBEDDING_PROVIDER=openrouter` `mistralai/codestral-embed-2505` `shared_cache_query_root_current_state_text_vector_batch/...q8_k4_r2_c6...dim1536_precbp5000_curbp5000_hitbp8750` | 706.33 µs | - | Vector rerank after the BM25/current-state candidate pass keeps the same seven-of-eight target hit shape and adds substantial dimension-sensitive cost. |
| `SELENE_EMBEDDING_PROVIDER=openrouter` `mistralai/codestral-embed-2505` `shared_cache_query_root_vector_text_batch/...q8_k4_r2_c4...dim1536_precbp6562_curbp5312_hitbp8750` | 327.96 µs | - | Reverse-order fusion: vector-expanded top-k candidates feed `selene.text_score_nodes_batch`. It is slower than maintained BM25 and current-state vector baselines on the same run, while keeping the same seven-of-eight target hit shape and lowering topic/current precision. |
| `SELENE_EMBEDDING_CORPUS=code_alias_wide_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c8...dim1536_precbp6250_curbp6250_hitbp8750` | 171.51 µs | - | Wider 16-query Codestral code/alias profile. BM25/current-state remains fastest but finds 14 of 16 expected target facts. |
| `SELENE_EMBEDDING_CORPUS=code_alias_wide_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c8...dim1536_precbp6250_curbp6250_hitbp8750` | 2.6202 ms | - | Vector rerank after BM25/current-state keeps the same target-hit and precision shape while adding substantial cost. |
| `SELENE_EMBEDDING_CORPUS=code_alias_wide_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c11...dim1536_precbp10000_hitbp9375` | 344.60 µs | - | Plain graph-expanded vector scoring finds 15 of 16 expected targets and keeps full broad-topic precision, but has no current-state gate. |
| `SELENE_EMBEDDING_CORPUS=code_alias_wide_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c8...dim1536_basecurbp7968_curbp10000_hitbp8750` | 331.99 µs | - | Maintained current-state vector scoring restores current precision but drops to the same 14-of-16 target-hit shape as BM25/current-state on this wider profile. |
| `SELENE_EMBEDDING_CORPUS=code_alias_wide_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c8...dim1536_basecurbp7968_curbp10000_hitbp8750` | 346.01 µs | - | Provenance-required current state preserves the same quality shape as exclusion-only current state with modest extra cost. |
| `SELENE_EMBEDDING_CORPUS=code_alias_wide_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp7031_curbp5781_hitbp9375` | 522.62 µs | - | Vector-first BM25 finds 15 of 16 expected targets like plain vector scoring, but loses broad/current precision and costs more than current-state vector scoring. |
| `SELENE_EMBEDDING_CORPUS=project_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c6...dim1536_precbp9531_curbp9531_hitbp10000` | 170.50 µs | - | Curated project-code profile using real selene-db module and symbol names. Maintained BM25/current-state is fastest and finds all expected source-shaped targets, with one off-topic/current miss across 64 returned rows. |
| `SELENE_EMBEDDING_CORPUS=project_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c6...dim1536_precbp9531_curbp9531_hitbp10000` | 3.1734 ms | - | Vector rerank after BM25/current-state preserves target hits and precision but adds a large exact-vector pass. |
| `SELENE_EMBEDDING_CORPUS=project_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c8...dim1536_precbp10000_hitbp10000` | 310.13 µs | - | Plain graph-expanded vector scoring reaches full topic precision and all 16 targets on the source-shaped corpus, but does not apply maintained current-state semantics. |
| `SELENE_EMBEDDING_CORPUS=project_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c6...dim1536_basecurbp10000_curbp10000_hitbp10000` | 306.32 µs | - | Maintained current-state vector scoring keeps full current precision and all expected targets, about 1.8x slower than BM25/current-state on this profile. |
| `SELENE_EMBEDDING_CORPUS=project_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c6...dim1536_basecurbp10000_curbp10000_hitbp10000` | 307.69 µs | - | Provenance-required current state preserves the same quality shape as current state with negligible extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp8750_curbp8750_hitbp10000` | 486.06 µs | - | Vector-first BM25 finds all expected targets after pruning to top-k vector candidates, but lowers broad/current precision and remains slower than either BM25/current-state or current-state vector scoring. |
| `SELENE_EMBEDDING_CORPUS=project_code_alias_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c8...dim1536_precbp9218_curbp9218_hitbp9375` | 170.28 µs | - | Harder source-shaped alias profile with lexical decoys. BM25/current-state remains fastest but misses one expected target. |
| `SELENE_EMBEDDING_CORPUS=project_code_alias_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c8...dim1536_precbp9218_curbp9218_hitbp9375` | 3.1401 ms | - | Vector rerank after BM25/current-state keeps the same 15-of-16 target-hit shape because the missing target is not in the lexical candidate set. |
| `SELENE_EMBEDDING_CORPUS=project_code_alias_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c10...dim1536_precbp10000_hitbp8750` | 318.59 µs | - | Plain graph-expanded vector scoring keeps full broad-topic precision but finds only 14 of 16 expected alias targets. |
| `SELENE_EMBEDDING_CORPUS=project_code_alias_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c8...dim1536_basecurbp9687_curbp10000_hitbp10000` | 314.32 µs | - | Maintained current-state vector scoring recovers all expected alias targets and restores full current precision. |
| `SELENE_EMBEDDING_CORPUS=project_code_alias_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c8...dim1536_basecurbp9687_curbp10000_hitbp10000` | 314.61 µs | - | Provenance-required current state preserves the current-state vector quality with negligible extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_code_alias_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp8906_curbp8593_hitbp8750` | 495.39 µs | - | Vector-first BM25 is again negative: it misses two targets, lowers current precision, and is slower than both BM25/current-state and current-state vector scoring. |
| `SELENE_EMBEDDING_CORPUS=project_source_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c6...dim1536_precbp9843_curbp9843_hitbp10000` | 195.52 µs | - | Real source-excerpt profile. Maintained BM25/current-state is target-complete and fastest, with one broad/current miss across 64 returned rows. |
| `SELENE_EMBEDDING_CORPUS=project_source_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c6...dim1536_precbp9843_curbp9843_hitbp10000` | 3.2922 ms | - | Vector rerank after target-complete BM25/current-state keeps the same quality shape and adds a large exact-vector pass. |
| `SELENE_EMBEDDING_CORPUS=project_source_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c10...dim1536_precbp10000_hitbp9375` | 344.62 µs | - | Plain graph-expanded vector scoring keeps full broad-topic precision but misses one expected source target and has no current-state gate. |
| `SELENE_EMBEDDING_CORPUS=project_source_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c6...dim1536_basecurbp8906_curbp10000_hitbp10000` | 328.21 µs | - | Maintained current-state vector scoring restores full current precision and remains target-complete on real source excerpts. |
| `SELENE_EMBEDDING_CORPUS=project_source_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c6...dim1536_basecurbp8906_curbp10000_hitbp10000` | 330.03 µs | - | Provenance-required current state preserves the current-state vector quality with negligible extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_source_code_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp9531_curbp8593_hitbp9375` | 527.43 µs | - | Vector-first BM25 misses one target, lowers current precision, and remains slower than both BM25/current-state and current-state vector scoring. |
| `SELENE_EMBEDDING_CORPUS=project_source_chunk_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c6...dim1536_precbp9375_curbp9375_hitbp10000` | 193.15 µs | - | Source-chunk profile with target-aware implementation snippets. Maintained BM25/current-state is fastest and target-complete, but has one broad/current miss across 64 returned rows. |
| `SELENE_EMBEDDING_CORPUS=project_source_chunk_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c6...dim1536_precbp9375_curbp9375_hitbp10000` | 3.2712 ms | - | Vector rerank after BM25/current-state keeps the same target/precision shape and adds a large exact-vector pass. |
| `SELENE_EMBEDDING_CORPUS=project_source_chunk_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c8...dim1536_precbp10000_hitbp10000` | 333.57 µs | - | Plain graph-expanded vector scoring reaches full target and broad-topic precision, but does not apply maintained current-state semantics. |
| `SELENE_EMBEDDING_CORPUS=project_source_chunk_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c6...dim1536_basecurbp9687_curbp10000_hitbp10000` | 328.45 µs | - | Maintained current-state vector scoring is target-complete and restores full current precision at about 1.7x BM25/current-state latency. |
| `SELENE_EMBEDDING_CORPUS=project_source_chunk_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c6...dim1536_basecurbp9687_curbp10000_hitbp10000` | 331.10 µs | - | Provenance-required current state preserves the current-state vector quality with minimal extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_source_chunk_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp9843_curbp9531_hitbp10000` | 523.56 µs | - | Vector-first BM25 is target-complete and has better broad precision than BM25/current-state on this chunk profile, but it remains slower than current-state vector and less current-precise. |
| `SELENE_EMBEDDING_CORPUS=project_source_file_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q8_k4_r2_c4...dim1536_precbp9375_curbp9375_hitbp10000` | 142.50 µs | - | File-level source corpus with selected real selene-db files. Maintained BM25/current-state is target-complete and fastest, with one broad/current miss across 32 returned rows. |
| `SELENE_EMBEDDING_CORPUS=project_source_file_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q8_k4_r2_c4...dim1536_precbp9375_curbp9375_hitbp10000` | 1.0473 ms | - | Vector rerank after BM25/current-state preserves quality but adds a large exact-vector pass even at q8/c4. |
| `SELENE_EMBEDDING_CORPUS=project_source_file_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q8_k4_r2_c6...dim1536_precbp10000_hitbp10000` | 210.97 µs | - | Plain graph-expanded vector scoring is target-complete and full topic precision, but has no current-state gate. |
| `SELENE_EMBEDDING_CORPUS=project_source_file_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q8_k4_r2_c4...dim1536_basecurbp10000_curbp10000_hitbp10000` | 212.75 µs | - | Maintained current-state vector scoring keeps full target/current precision, about 1.5x slower than BM25/current-state. |
| `SELENE_EMBEDDING_CORPUS=project_source_file_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q8_k4_r2_c4...dim1536_basecurbp10000_curbp10000_hitbp10000` | 213.11 µs | - | Provenance-required current state preserves the current-state vector quality with negligible extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_source_file_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q8_k4_r2_c4...dim1536_precbp10000_curbp10000_hitbp10000` | 318.20 µs | - | Vector-first BM25 is finally full-quality on this file-level corpus, but it is still slower than current-state vector scoring and much slower than BM25/current-state. |
| `SELENE_EMBEDDING_CORPUS=project_workspace_source_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c4...dim1536_precbp9375_curbp9375_hitbp10000` | 171.90 µs | - | Live workspace-source corpus with line-numbered source windows from the current checkout. Maintained BM25/current-state is fastest and target-complete, with one broad/current miss across 64 returned rows. |
| `SELENE_EMBEDDING_CORPUS=project_workspace_source_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c4...dim1536_precbp9375_curbp9375_hitbp10000` | 3.2837 ms | - | Vector rerank after BM25/current-state preserves the same quality shape because the lexical candidate producer was already target-complete, but adds a large exact-vector pass. |
| `SELENE_EMBEDDING_CORPUS=project_workspace_source_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c6...dim1536_precbp10000_hitbp10000` | 304.22 µs | - | Plain graph-expanded vector scoring is target-complete and full topic precision, but has no maintained current-state gate. |
| `SELENE_EMBEDDING_CORPUS=project_workspace_source_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c4...dim1536_basecurbp10000_curbp10000_hitbp10000` | 301.24 µs | - | Maintained current-state vector scoring keeps full target/current precision, about 1.75x slower than BM25/current-state on this live-source profile. |
| `SELENE_EMBEDDING_CORPUS=project_workspace_source_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c4...dim1536_basecurbp10000_curbp10000_hitbp10000` | 302.39 µs | - | Provenance-required current state preserves the current-state vector quality with negligible extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_workspace_source_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp10000_curbp10000_hitbp10000` | 492.16 µs | - | Vector-first BM25 is full-quality on live source windows, but still slower than current-state vector scoring and much slower than BM25/current-state. |
| `SELENE_EMBEDDING_CORPUS=project_migration_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_score_batch/...q16_k4_r2_c4...dim1536_precbp9531_curbp9531_hitbp10000` | 170.21 µs | - | Legacy-alias migration profile with stale archived decoys. BM25/current-state is fastest and target-complete, but still has one broad/current miss across 64 returned rows. |
| `SELENE_EMBEDDING_CORPUS=project_migration_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_text_vector_batch/...q16_k4_r2_c4...dim1536_precbp9531_curbp9531_hitbp10000` | 3.1982 ms | - | Vector rerank after BM25/current-state preserves the same quality shape and adds a large exact-vector pass. |
| `SELENE_EMBEDDING_CORPUS=project_migration_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_expansion_batch/...q16_k4_r2_c8...dim1536_precbp10000_hitbp10000` | 307.72 µs | - | Plain graph-expanded vector scoring is target-complete and full broad-topic precision, but has no maintained current-state gate. |
| `SELENE_EMBEDDING_CORPUS=project_migration_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c4...dim1536_basecurbp8125_curbp10000_hitbp10000` | 295.29 µs | - | Maintained current-state vector scoring filters stale migration decoys, restores full current precision, and keeps all expected targets. |
| `SELENE_EMBEDDING_CORPUS=project_migration_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c4...dim1536_basecurbp8125_curbp10000_hitbp10000` | 295.72 µs | - | Provenance-required current state preserves the current-state vector quality with negligible extra cost. |
| `SELENE_EMBEDDING_CORPUS=project_migration_memory` `SELENE_EMBEDDING_PROVIDER=openrouter` `shared_cache_query_root_vector_text_batch/...q16_k4_r2_c4...dim1536_precbp10000_curbp8125_hitbp10000` | 493.91 µs | - | Vector-first BM25 is target-complete but admits stale/current-invalid hits, so it is slower and less current-precise than maintained current-state vector scoring. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_state_intersection/...q16_k4_r2_c14...precbp10000` | 1.55 ms | 1.62 ms | Same GQL-produced roots, then `selene.vector_score_candidate_state_expanded` intersects graph expansion with maintained `omlx_support_facts`, filtering root hint docs while preserving topic precision. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_root_state_intersection/...q16_k4_r2_c14...precbp10000` | 1.56 ms | 1.61 ms | Warmed full-plan-cache support-state scorer; unchanged within local noise versus the fresh-session row. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_current_state_intersection/...q16_k4_r2_c13...basecurbp8593/8281_curbp10000` | 1.34 ms | 1.41 ms | Intersects the same expanded roots with maintained `omlx_current_support_facts`, excluding graph-authored negative evidence and restoring full current-fact precision with one fewer first-query candidate. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_root_current_state_intersection/...q16_k4_r2_c13...basecurbp8593/8281_curbp10000` | 1.35 ms | 1.41 ms | Warmed full-plan-cache current-state scorer; still dominated by graph expansion plus vector rerank. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_provenance_state_intersection/...q16_k4_r2_c13...basecurbp8593/8281_curbp10000` | 1.67 ms | 1.64 ms | Intersects expanded roots with `omlx_provenance_current_support_facts`, requiring both incoming support and outgoing provenance edges while preserving the same candidate width and full current precision. |
| `procedure_vector_omlx_query_roots/shared_session_plan_cache_query_root_provenance_state_intersection/...q16_k4_r2_c13...basecurbp8593/8281_curbp10000` | 1.60 ms | 1.63 ms | Warmed full-plan-cache provenance-state scorer; positive edge-evidence checks add modest overhead versus exclusion-only current state on this quick local pass. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_expansion_batch/...q16_k4_r2_c16...precbp10000` | 312.49 µs | 528.84 µs | Single GQL statement builds all 16 query vectors and root sets from graph rows, then calls the batched expanded scorer once; avoids repeated statement/session overhead while preserving full topic precision. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_current_state_intersection_batch/...q16_k4_r2_c13...basecurbp8593/8281_curbp10000` | 284.24 µs | 491.37 µs | Single GQL statement builds all 16 query vectors/root sets, then calls the batched maintained-state expanded scorer once; restores full current precision while avoiding repeated session/procedure overhead. |
| `procedure_vector_omlx_query_roots/shared_cache_query_root_provenance_state_intersection_batch/...q16_k4_r2_c13...basecurbp8593/8281_curbp10000` | 294.60 µs | 496.62 µs | Batched provenance-gated current-state scorer; required support/provenance edge checks add little overhead once the 16-query shape is inside one procedure call. |

The opt-in `Qwen3-Embedding-8B-4bit-DWQ` local model also works on
`/v1/embeddings` and returns 4096-dimensional vectors. With
`SELENE_OMLX_EMBEDDING_MODELS=Qwen3-Embedding-8B-4bit-DWQ`, the cached
partial-hint expansion row reaches `precbp10000`, `c16`, at 205.26 us on the
same scaled profile, the maintained `omlx_support_facts` state row reaches
`precbp10000`, `c14`, at 184.50 us, and the ANN-root maintained-state row only
reaches `precbp5625`, `c42`, at 1.113 ms. The full GQL query-root expansion row
looks up anchors at 477.65 us repeated / 38.66 us hot-plan-cache / 40.96 us
batched, returns root rows at 568.42 us repeated / 54.44 us batched,
materializes roots at 708.65 us repeated / 52.94 us hot-plan-cache / 65.45 us
batched, reaches `precbp10000`, `r2`, `c16`, at 1.39 ms repeated and 1.38 ms
hot-plan-cache but only `basecurbp8437`, the GQL maintained-state intersection
row reaches `precbp10000`, `r2`, `c14`, at 1.47 ms repeated and 1.48 ms
hot-plan-cache, the negative-evidence current-state row reaches `curbp10000`,
`r2`, `c13`, at 1.47 ms repeated and 1.46 ms hot-plan-cache, and the GQL
batched text-score row reaches `precbp10000` / `curbp9218`, `r2`, `c16`, at
476.29 us. The GQL current-state BM25 expanded batch row reaches
`precbp10000` / `curbp10000`, `r2`, `c13`, at 185.55 us. Adding vector rerank
after that BM25 current-state candidate pass keeps the same quality but costs
8.6352 ms, so the GQL batched expansion row at `precbp10000`, `r2`, `c16`, and
660.94 µs remains a better vector path. On the target-aware
`code_alias_memory` profile, current-state BM25 batch reaches `hitbp8750` at
135.41 us, while adding vector rerank keeps the same `hitbp8750` and costs
1.5738 ms. The plain graph-expanded vector batch reaches `precbp10000` and
`hitbp8750`, `r2`, `c9`, at 336.08 us; the maintained vector current-state
batch row reaches `hitbp10000`, `basecurbp8125` / `curbp10000`, `r2`, `c6`, at
345.56 us, making maintained current-state composition the better
target-quality path for that code/alias profile. The vector
batched current-state row reaches `curbp10000`, `r2`, `c13`, at 719.86 µs, and
the batched provenance-state row reaches the same
`curbp10000` / `c13` shape at 707.24 µs. The conservative cached r60/w40 mixed
cycle is 12.52 ms on the same 4096-dimensional row. It stays opt-in for now so
default local oMLX rows remain short and comparable to the earlier two-model
baseline.

OpenRouter `mistralai/codestral-embed-2505` is available through
`SELENE_EMBEDDING_PROVIDER=openrouter` and returns 1536-dimensional vectors. On
the same target-aware `code_alias_memory` profile, the plain graph-expanded
batch row reaches `precbp10000` / `hitbp8750`, `r2`, `c9`, at 211.05 us, while
the maintained current-state vector batch reaches `hitbp10000`,
`basecurbp7812` / `curbp10000`, `r2`, `c6`, at 211.95 us. Provenance-gated
current-state scoring keeps the same target quality at 212.38 us. Maintained
BM25 over the same graph-expanded current set is faster at 136.79 us but still
misses one expected code/alias target, and vector rerank after that BM25 pass
does not improve target hits while costing 706.33 us. The reverse order,
vector-expanded top-k followed by BM25 over explicit candidates, reaches only
`precbp6562` / `curbp5312` / `hitbp8750` at 327.96 us. This keeps the
current-state conclusion intact for the code-specialized embedding model:
state composition, not model choice alone, recovers the missing target; BM25 is
the fast lexical path only when its candidate producer is target-complete, and
vector-first BM25 pruning is not a win on this corpus.

On the wider `code_alias_wide_memory` profile, target-hit splits further:
current-state BM25 and current-state vector rows find 14 of 16 targets
(`hitbp8750`), while plain vector expansion and vector-first BM25 find 15 of 16
(`hitbp9375`). That extra target comes with a currentness/precision tradeoff:
vector-first BM25 falls to `precbp7031` / `curbp5781` and costs 522.62 us, so
the better follow-up is richer graph state/corpus analysis rather than a fused
procedure.

On the `project_code_memory` profile, all measured Codestral paths find all 16
expected source-shaped targets. BM25/current-state is the fastest path
(`precbp9531` / `curbp9531` / `hitbp10000`, 170.50 us), while current-state
vector scoring restores full broad/current precision at 306.32 us. Vector-first
BM25 keeps `hitbp10000`, but drops to `precbp8750` / `curbp8750` and costs
486.06 us, so it remains a negative default-fusion result even on a corpus where
vector roots are target-complete.

On the harder `project_code_alias_memory` profile, lexical decoys create the
target-quality gap the positive control lacked. BM25/current-state is still
fastest but finds 15 of 16 targets (`hitbp9375`, 170.28 us), and vector rerank
after those BM25 candidates cannot recover the missing target. Current-state
vector scoring reaches `hitbp10000` and `curbp10000` at 314.32 us, making it
the quality path for this source-alias fixture. Vector-first BM25 stays
negative: `hitbp8750`, `curbp8593`, and 495.39 us.

On the `project_source_code_memory` profile, short real source excerpts make
BM25/current-state target-complete again (`hitbp10000`, 195.52 us), so this
acts as a source-file positive control rather than another lexical-failure
case. Maintained current-state vector scoring is also target-complete and
restores `curbp10000` at 328.21 us, while plain graph-expanded vector and
vector-first BM25 both miss one target (`hitbp9375`). The conclusion remains
compositional: use maintained graph state plus the scoring primitive that wins
the corpus, and do not add a fused vector-first BM25 API from these rows.

On the `project_source_chunk_memory` profile, target-aware implementation
snippets keep all measured paths target-complete after the graph-root layout is
kept target-free. BM25/current-state is still fastest (`hitbp10000`,
193.15 us) but has one broad/current miss. Current-state vector scoring
restores `curbp10000` at 328.45 us. Vector-first BM25 is also target-complete
and improves broad precision (`precbp9843`) over BM25/current-state, but it
costs 523.56 us and remains below current-state vector on current precision.
This is useful A/B evidence for chunked code retrieval, not a reason to add a
fused vector-first text API.

On the `project_source_file_memory` profile, selected real project files are
embedded as whole file-level documents. BM25/current-state remains the fastest
target-complete path (`hitbp10000`, 142.50 us) with one broad/current miss.
Current-state vector scoring restores full broad/current precision at
212.75 us. Vector-first BM25 also reaches full target/current precision here
(`hitbp10000`, 318.20 us), which makes this the first positive quality row for
that reverse order, but it is still slower than current-state vector scoring
and much slower than BM25/current-state. Keep it as evidence for corpus-shaped
A/B testing, not production API promotion.

On the `project_migration_memory` profile, queries use archived/prototype
terminology while current target documents cite first-class engine surfaces and
stale decoys model old-code drift. BM25/current-state is still fastest and
target-complete (`hitbp10000`, 170.21 us), but has one broad/current miss.
Current-state vector scoring restores full current precision at 295.29 us, and
vector-first BM25 is target-complete but current-imprecise
(`curbp8125`, 493.91 us). This reinforces graph-maintained current state as the
right correction for old-code drift; vector-first text remains a benchmarked
composition choice, not a default API.

The locally listed `jina-code-embeddings-1.5b-mlx` model is not currently
available from `/v1/embeddings` in oMLX; the endpoint returns HTTP 400 and
reports it as an LLM model for `/v1/chat/completions`, so it is not part of
these rows until it is exposed as an embedding model.

Component-pressure rows pool the query component with additional graph
components before exact vector scoring. Quality remains perfect on this clean
fixture, so these rows isolate candidate-set size pressure rather than topology
noise:

| Component pool | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_component_pressure/component_pool_w1/...c16_covbp10000_curbp10000_precbp10000` | 53.0 µs | 116.3 µs | Scores only the query anchor's compact component; this is the same primitive direction as WCC component filtering. |
| `graph_vector_component_pressure/component_pool_w4/...c64_covbp10000_curbp10000_precbp10000` | 183.4 µs | 466.8 µs | Four pooled components still beat broad graph expansion at 10k while keeping oracle quality. |
| `graph_vector_component_pressure/component_pool_w16/...c256_covbp10000_curbp10000_precbp10000` | 685.6 µs | 1.550 ms | Candidate scoring remains linear and predictable, but it is now in the same range as wide graph expansion. |
| `graph_vector_component_pressure/component_pool_w64/...covbp10000_curbp10000_precbp10000` | 2.689 ms (`w62`, `c992`) | 5.691 ms (`c976`) | Near-global pooled scoring is still exact and high quality, but too expensive for the default graph-filtered path; this is the fallback point for ANN or compressed pre-scoring research. |

Topology-pressure rows add cross-topic `SUPPORTS` noise before WCC projection,
then compare the broadened WCC component against a hard topic/session-style
candidate filter. Quality remains perfect because exact vector scoring can
still recover the right topic, so this isolates topology noise as candidate-set
inflation:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_topology_pressure/noisy_wcc/...covbp10000_curbp10000_precbp10000` | 2.535 ms (`c992`) | 56.68 ms (`c9728`) | Cross-topic topology noise collapses WCC into a near-global candidate set; exact scoring preserves quality but is far too expensive. |
| `graph_vector_topology_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 102.7 µs (`c32`) | 1.109 ms (`c152`) | A hard topic/session candidate set restores bounded exact scoring under the same noisy graph, pointing toward query-derived subgraph filters before ANN/PQ fallback. |

Community-pressure rows compare algorithm-derived community partitions against
the same noisy topology. These rows are benchmark-only and intentionally use the
same exact vector scorer/fact-diverse selector, so they isolate partition
quality rather than rerank behavior:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_community_pressure/noisy_wcc/...covbp10000_curbp10000_precbp10000` | 2.534 ms (`c992`) | 56.11 ms (`c9728`) | Same broad WCC baseline as topology pressure; quality is perfect only because exact scoring scans nearly the whole noisy graph. |
| `graph_vector_community_pressure/louvain/...` | 7.08 µs (`c2`, `covbp2500`) | 15.59 µs (`c2`, `covbp1250`) | Single-pass Louvain over this star-like memory topology over-partitions to tiny candidate sets. It is fast, but loses too much coverage to be a useful default partition source here. |
| `graph_vector_community_pressure/label_propagation/...` | 53.34 µs (`c17`, `covbp7661`, `precbp8830`) | 112.0 µs (`c16`, `covbp7578`, `precbp8789`) | Label propagation is a useful middle row: compact and fast, but partial-recall compared with the hard topic/session filter. |
| `graph_vector_community_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 102.6 µs (`c32`) | 1.094 ms (`c152`) | The hard topic/session filter remains the full-quality reference under this noisy graph. Next research should derive comparable filters from graph/query structure rather than trusting connectivity alone. |

Query-filter rows replace the metadata-only hard topic candidate set with graph
candidate production. Each memory node links to a scope node via `IN_SCOPE`;
the query path follows the anchor's scope edge, scans incoming scope membership
edges, then exact-scores those graph-derived candidates:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_query_filter_pressure/noisy_wcc/...covbp10000_curbp10000_precbp10000` | 2.558 ms (`c992`) | 56.61 ms (`c9728`) | Repeats the noisy WCC baseline inside the query-filter fixture. |
| `graph_vector_query_filter_pressure/label_propagation/...` | 54.86 µs (`c17`, `covbp7661`, `precbp8830`) | 114.3 µs (`c16`, `covbp7578`, `precbp8789`) | Label propagation stays compact and partial-recall when compared with graph-derived scope filtering. |
| `graph_vector_query_filter_pressure/graph_scope_filter/...covbp10000_curbp10000_precbp10000` | 111.2 µs (`c32`) | 1.142 ms (`c152`) | Graph-derived scope membership matches hard-topic quality with a small traversal overhead. This is the strongest product-shaped primitive so far: graph/query candidate production plus exact vector scoring. |
| `graph_vector_query_filter_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 104.3 µs (`c32`) | 1.102 ms (`c152`) | Metadata hard-topic filtering remains the lower-bound reference for the same candidate set. |
| `graph_vector_query_filter_pressure/graph_scope_candidate_set_batch_score/...covbp10000_curbp10000_precbp10000` | 121.8 µs (`c32`) | 1.238 ms (`c152`) | Canonical `VectorCandidateSet` batch scoring over graph-query output preserves quality but adds normalization/batch overhead when no second set is composed. |
| `graph_vector_query_filter_pressure/graph_scope_unresolved_current_algebra_batch_score/...covbp10000_curbp10000_precbp10000` | 80.92 µs (`c18`) | 840.4 µs (`c89`) | Composing graph-scope output with the maintained unresolved-current candidate set cuts exact scoring work while preserving full quality. |

The candidate-set rows show the boundary tradeoff: canonical graph-query output
is not faster by itself, but it becomes valuable once it composes with another
maintained graph-derived set. That supports `VectorCandidateSet` as a Rust-side
graph/query/active-set glue primitive before adding narrower procedure surfaces.

Session-filter rows add a coarser graph-derived membership edge. Each memory
node links to an `IN_SESSION` node shared by four topics, modeling task/session
scope that is broader than an exact topic but much narrower than noisy
connectivity. These rows keep the same noisy topology and exact vector scorer.
The `*_current_filter` variants prune stale fixture metadata before scoring,
mirroring a freshness-aware graph candidate producer instead of relying on
post-score current-result selection. The `*_unsuperseded_filter` variants derive
the same current candidate set from graph topology by rejecting nodes with an
outgoing `SUPERSEDED_BY` edge before scoring. The
`*_materialized_current_filter` variants use the same graph-derived current set
materialized once during fixture setup. The `*_provenance_expand` variants then
score only graph-current provenance roots with outgoing `SUPPORTS` edges and use
graph expansion plus `SUPERSEDED_BY` repair to recover current supporting facts.
The `*_k1` variants expand only the nearest provenance root, while the
non-suffixed provenance rows expand four roots:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_session_filter_pressure/noisy_wcc/...covbp10000_curbp10000_precbp10000` | 2.818 ms (`c992`) | 60.41 ms (`c9728`) | Repeats the noisy WCC baseline with session edges present outside the WCC projection. |
| `graph_vector_session_filter_pressure/label_propagation/...` | 60.11 µs (`c17`, `covbp7661`, `precbp8830`) | 123.8 µs (`c16`, `covbp7578`, `precbp8789`) | Label propagation remains fast but partial-recall against the full-quality graph membership filters. |
| `graph_vector_session_filter_pressure/graph_session_filter/...covbp10000_curbp10000_precbp10000` | 381.2 µs (`c124`) | 3.781 ms (`c608`) | Four-topic session membership is a useful middle row: much cheaper than noisy WCC and full-quality, but about 3x the exact scope filter at 10k. |
| `graph_vector_session_filter_pressure/graph_session_current_filter/...covbp10000_curbp10000_precbp10000` | 243.3 µs (`c70`) | 2.588 ms (`c356`) | Pre-score metadata freshness pruning keeps full quality while cutting session candidate pressure by roughly half. |
| `graph_vector_session_filter_pressure/graph_session_unsuperseded_filter/...covbp10000_curbp10000_precbp10000` | 291.6 µs (`c70`) | 3.154 ms (`c356`) | Graph-derived freshness reaches the same candidate count and quality as metadata current filtering, with extra edge-check overhead. |
| `graph_vector_session_filter_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 245.6 µs (`c70`) | 2.615 ms (`c356`) | Materialized graph-derived currentness recovers nearly all metadata-current latency while avoiding per-candidate edge scans. |
| `graph_vector_session_filter_pressure/graph_session_provenance_expand_k1/...covbp10000_curbp10000_precbp10000` | 132.3 µs (`c15`) | 1.313 ms (`c76`) | Expanding only the nearest session provenance root is enough for full grounded coverage on this fixture. |
| `graph_vector_session_filter_pressure/graph_session_provenance_expand/...covbp10000_curbp10000_precbp10000` | 151.3 µs (`c15`) | 1.347 ms (`c76`) | Four-root session provenance expansion preserves quality but adds graph expansion work over the k1 row. |
| `graph_vector_session_filter_pressure/graph_scope_filter/...covbp10000_curbp10000_precbp10000` | 120.9 µs (`c32`) | 1.236 ms (`c152`) | Exact graph scope remains the best product-shaped candidate filter when the query can identify a narrow subgraph. |
| `graph_vector_session_filter_pressure/graph_scope_current_filter/...covbp10000_curbp10000_precbp10000` | 79.50 µs (`c18`) | 824.6 µs (`c89`) | Freshness-aware scope filtering preserves full quality and lowers exact scoring work below the metadata topic baseline. |
| `graph_vector_session_filter_pressure/graph_scope_unsuperseded_filter/...covbp10000_curbp10000_precbp10000` | 92.23 µs (`c18`) | 989.6 µs (`c89`) | Graph-derived scope freshness is still faster than raw scope scoring while avoiding fixture-side current metadata. |
| `graph_vector_session_filter_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 80.08 µs (`c18`) | 839.1 µs (`c89`) | Materialized graph-derived scope currentness stays close to metadata current filtering and below the hard-topic lower-bound row. |
| `graph_vector_session_filter_pressure/graph_scope_provenance_expand_k1/...covbp10000_curbp10000_precbp10000` | 53.18 µs (`c4`) | 390.4 µs (`c19`) | One scope-local provenance root reaches full quality and is now the lowest full-quality graph-derived row. |
| `graph_vector_session_filter_pressure/graph_scope_provenance_expand/...covbp10000_curbp10000_precbp10000` | 71.85 µs (`c4`) | 429.6 µs (`c19`) | Four-root scope provenance expansion keeps full quality but is slower than k1 on this fixture. |
| `graph_vector_session_filter_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 114.0 µs (`c32`) | 1.190 ms (`c152`) | Metadata hard-topic filtering remains the lower-bound reference for the same narrow candidate set. |

Sparse-provenance rows use the same scope/session candidate producers, but each
summary provenance root supports only a partition of the topic facts. This
turns provenance root fanout into a measurable quality/latency knob instead of
letting a single nearest root cover the whole support set:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_sparse_provenance_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 371.6 µs (`c113`) | 3.854 ms (`c552`) | Full-quality broad session-current baseline. |
| `graph_vector_sparse_provenance_pressure/graph_session_provenance_expand_k1/...covbp3750_curbp3750_precbp3750` | 146.4 µs (`c15`) | 1.452 ms (`c76`) | Fast, but one sparse provenance root covers only 3750 bp of the supporting facts. |
| `graph_vector_sparse_provenance_pressure/graph_session_provenance_expand/...` | 165.9 µs (`c15`) | 1.516 ms (`c76`, `covbp8144`) | Four roots recover full quality at 1k, but not at the larger session scale. |
| `graph_vector_sparse_provenance_pressure/graph_session_provenance_expand_k8/...` | 172.8 µs (`c15`) | 1.545 ms (`c76`, `covbp9453`) | Eight roots nearly close the 10k quality gap with a small latency increase. |
| `graph_vector_sparse_provenance_pressure/graph_session_provenance_expand_k16/...covbp10000_curbp10000_precbp10000` | 186.2 µs (`c15`) | 1.578 ms (`c76`) | Sixteen roots reach full quality while staying roughly 2.4x faster than materialized session-current scoring. |
| `graph_vector_sparse_provenance_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 116.7 µs (`c29`) | 1.243 ms (`c138`) | Full-quality graph-scope current baseline. |
| `graph_vector_sparse_provenance_pressure/graph_scope_provenance_expand_k1/...covbp3750_curbp3750_precbp3750` | 44.67 µs (`c4`) | 408.6 µs (`c19`) | The lowest-latency graph-derived row, but only partial support coverage. |
| `graph_vector_sparse_provenance_pressure/graph_scope_provenance_expand/...` | 63.41 µs (`c4`) | 452.9 µs (`c19`, `covbp8144`) | Four roots are enough at 1k, but still partial at 10k. |
| `graph_vector_sparse_provenance_pressure/graph_scope_provenance_expand_k8/...` | 63.49 µs (`c4`) | 489.6 µs (`c19`, `covbp9453`) | Eight roots improve 10k support coverage with little 1k cost. |
| `graph_vector_sparse_provenance_pressure/graph_scope_provenance_expand_k16/...covbp10000_curbp10000_precbp10000` | 64.26 µs (`c4`) | 532.6 µs (`c19`) | Sixteen roots reach full quality and stay faster than the hard-topic metadata reference. |
| `graph_vector_sparse_provenance_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 113.8 µs (`c32`) | 1.227 ms (`c152`) | Metadata hard-topic lower-bound reference for comparison. |

Noisy sparse-provenance rows add current wrong-topic `SUPPORTS` edges before the
same sparse correct support partitions. This tests whether a graph-expansion
policy can tolerate plausible cross-topic provenance noise:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_noisy_sparse_provenance_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 374.2 µs (`c113`) | 3.735 ms (`c552`) | Full-quality broad session-current baseline under noisy support topology. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_session_provenance_expand_k1/...covbp3750_curbp3750_precbp3750` | 143.7 µs (`c15`) | 1.434 ms (`c76`) | One root is fast but now loses both coverage and precision because wrong-topic support fills the tail. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_session_provenance_expand/...` | 165.4 µs (`c15`, full) | 1.481 ms (`c76`, `covbp8144`, `precbp8906`) | Four roots recover full 1k quality but stay partial at 10k and admit noisy evidence. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_session_provenance_expand_k8/...` | 172.9 µs (`c15`, full) | 1.518 ms (`c76`, `covbp9453`, `precbp9648`) | Eight roots nearly close the 10k gap but still leave some cross-topic tail results. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_session_provenance_expand_k16/...covbp10000_curbp10000_precbp10000` | 191.3 µs (`c15`) | 1.569 ms (`c76`) | Sixteen roots restore full quality while remaining materially faster than materialized session-current scoring. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 116.4 µs (`c29`) | 1.205 ms (`c138`) | Full-quality scope-current baseline. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_scope_provenance_expand_k1/...covbp3750_curbp3750_precbp3750` | 44.11 µs (`c4`) | 405.1 µs (`c19`) | Lowest latency, but same coverage/precision failure as the session k1 row. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_scope_provenance_expand/...` | 62.14 µs (`c4`, full) | 446.9 µs (`c19`, `covbp8144`, `precbp8906`) | Scope-local four-root expansion is faster than session expansion but has the same 10k quality loss. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_scope_provenance_expand_k8/...` | 62.49 µs (`c4`, full) | 486.3 µs (`c19`, `covbp9453`, `precbp9648`) | Eight roots improve noisy support coverage and precision, but do not fully close them at 10k. |
| `graph_vector_noisy_sparse_provenance_pressure/graph_scope_provenance_expand_k16/...covbp10000_curbp10000_precbp10000` | 61.98 µs (`c4`) | 533.4 µs (`c19`) | Sixteen roots reach full noisy-support quality and remain faster than the topic-filter reference. |
| `graph_vector_noisy_sparse_provenance_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 113.6 µs (`c32`) | 1.190 ms (`c152`) | Metadata hard-topic lower-bound reference for comparison. |

Multi-hop provenance rows route half of each summary root's support facts
through `MemoryBridge` nodes. The one-hop row intentionally misses those bridged
facts; the two-hop row follows one more `SUPPORTS` layer:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_multihop_provenance_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 249.7 µs (`c70`) | 2.688 ms (`c356`) | Full-quality broad session-current baseline. |
| `graph_vector_multihop_provenance_pressure/graph_session_provenance_expand_k1/...covbp5000_curbp5000_precbp5000` | 128.9 µs (`c15`) | 1.390 ms (`c76`) | One-hop expansion reaches only the direct half of each root's support set. |
| `graph_vector_multihop_provenance_pressure/graph_session_provenance_expand_2hop_k1/...covbp10000_curbp10000_precbp10000` | 141.7 µs (`c15`) | 1.423 ms (`c76`) | Two-hop expansion restores full quality with a small extra traversal cost. |
| `graph_vector_multihop_provenance_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 80.29 µs (`c18`) | 853.0 µs (`c89`) | Full-quality scope-current baseline. |
| `graph_vector_multihop_provenance_pressure/graph_scope_provenance_expand_k1/...covbp5000_curbp5000_precbp5000` | 44.97 µs (`c4`) | 391.6 µs (`c19`) | Scope-local one-hop expansion has the same half-coverage failure. |
| `graph_vector_multihop_provenance_pressure/graph_scope_provenance_expand_2hop_k1/...covbp10000_curbp10000_precbp10000` | 58.43 µs (`c4`) | 424.9 µs (`c19`) | Two-hop scope expansion restores full quality and stays below both materialized-current and topic-filter references. |
| `graph_vector_multihop_provenance_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 113.5 µs (`c32`) | 1.218 ms (`c152`) | Metadata hard-topic lower-bound reference for comparison. |

Noisy multi-hop provenance rows add one current wrong-topic `SUPPORTS` edge
before the same bridged support pattern. This keeps the full correct support
set but tests whether bounded-depth expansion admits off-topic tail evidence:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_noisy_multihop_provenance_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 248.4 µs (`c70`) | 2.677 ms (`c356`) | Full-quality broad session-current baseline under noisy bridged support. |
| `graph_vector_noisy_multihop_provenance_pressure/graph_session_provenance_expand_k1/...covbp5000_curbp5000_precbp5000` | 127.8 µs (`c15`) | 1.402 ms (`c76`) | One-hop expansion still reaches only the direct half; off-topic tail evidence does not improve coverage. |
| `graph_vector_noisy_multihop_provenance_pressure/graph_session_provenance_expand_2hop_k1/...covbp10000_curbp10000_precbp10000` | 141.7 µs (`c15`) | 1.433 ms (`c76`) | Two-hop expansion restores full quality before deferred off-topic support can enter the final result set. |
| `graph_vector_noisy_multihop_provenance_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 80.09 µs (`c18`) | 851.0 µs (`c89`) | Full-quality scope-current baseline. |
| `graph_vector_noisy_multihop_provenance_pressure/graph_scope_provenance_expand_k1/...covbp5000_curbp5000_precbp5000` | 45.76 µs (`c4`) | 400.2 µs (`c19`) | Scope-local one-hop expansion has the same half-coverage failure. |
| `graph_vector_noisy_multihop_provenance_pressure/graph_scope_provenance_expand_2hop_k1/...covbp10000_curbp10000_precbp10000` | 59.09 µs (`c4`) | 432.1 µs (`c19`) | Two-hop scope expansion keeps full quality and stays below both materialized-current and topic-filter references. |
| `graph_vector_noisy_multihop_provenance_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 113.6 µs (`c32`) | 1.209 ms (`c152`) | Metadata hard-topic lower-bound reference for comparison. |

Noisy sparse multi-hop provenance rows combine sparse correct support,
wrong-topic support inserted first, and bridged correct support facts. This is
the first fixture where provenance depth and root fanout matter together:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 373.6 µs (`c113`) | 3.828 ms (`c552`) | Full-quality broad session-current baseline. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_provenance_expand_k1/...covbp2500_curbp2500_precbp2500` | 145.2 µs (`c15`) | 1.475 ms (`c76`) | One-hop k1 sees only the direct part of one sparse root. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_provenance_expand_2hop_k1/...covbp3750_curbp3750_precbp3750` | 148.8 µs (`c15`) | 1.467 ms (`c76`) | Two-hop k1 recovers bridged facts for one root but still leaves sparse-root coverage on the floor. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_provenance_expand_2hop/...` | 175.4 µs (`c15`, full) | 1.521 ms (`c76`, `covbp8144`, `precbp8886`) | Four two-hop roots are enough at 1k but partial at 10k under sparse noisy support. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_provenance_expand_2hop_k8/...` | 193.8 µs (`c15`, full) | 1.583 ms (`c76`, `covbp9453`, `precbp9648`) | Eight two-hop roots nearly close the 10k gap. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_provenance_expand_k16/...` | 204.6 µs (`c15`, `covbp5927`, `precbp7500`) | 1.624 ms (`c76`, `covbp5000`, `precbp7500`) | Wide one-hop fanout improves precision but cannot see bridged correct support. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_session_provenance_expand_2hop_k16/...covbp10000_curbp10000_precbp10000` | 228.7 µs (`c15`) | 1.669 ms (`c76`) | Wide two-hop expansion restores full session quality, still below materialized-current scoring. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 116.6 µs (`c29`) | 1.242 ms (`c138`) | Full-quality scope-current baseline. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_provenance_expand_k1/...covbp2500_curbp2500_precbp2500` | 44.09 µs (`c4`) | 411.0 µs (`c19`) | Lowest latency but only the direct part of one sparse root. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_provenance_expand_2hop_k1/...covbp3750_curbp3750_precbp3750` | 47.17 µs (`c4`) | 418.1 µs (`c19`) | Two-hop k1 restores bridged facts for one root. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_provenance_expand_2hop/...` | 74.66 µs (`c4`, full) | 477.7 µs (`c19`, `covbp8144`, `precbp8886`) | Scope-local four-root expansion has the same 10k quality knee as the session row. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_provenance_expand_2hop_k8/...` | 74.16 µs (`c4`, full) | 540.8 µs (`c19`, `covbp9453`, `precbp9648`) | Eight roots nearly close the 10k gap while staying below topic filtering. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_provenance_expand_k16/...` | 60.06 µs (`c4`, `covbp5000`, `precbp7500`) | 571.0 µs (`c19`, `covbp5000`, `precbp7500`) | Wide one-hop fanout remains depth-limited. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/graph_scope_provenance_expand_2hop_k16/...covbp10000_curbp10000_precbp10000` | 74.23 µs (`c4`) | 615.2 µs (`c19`) | Wide two-hop scope expansion restores full quality and remains below materialized-current and topic-filter references. |
| `graph_vector_noisy_sparse_multihop_provenance_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 114.2 µs (`c32`) | 1.214 ms (`c152`) | Metadata hard-topic lower-bound reference for comparison. |

Active-subgraph composition rows reuse the noisy sparse multi-hop topology and
add `CONTRADICTS` edges to current duplicates. The new unresolved-provenance
rows intentionally intersect session/scope, unresolved-current active set,
provenance roots, two-hop expansion, and unresolved-current selection. That is
too strict on this fixture: it is fast, but loses recall because too few
unresolved roots remain:

| Bench | 9k/10k scale | Notes |
|---|---:|---|
| `graph_vector_active_subgraph_composition_pressure/graph_session_materialized_current_filter/9k_q64_c552_covbp10000_curbp10000_precbp10000` | 3.438 ms (quick) | Full-quality broad session-current baseline under noisy sparse multi-hop plus contradictions. |
| `graph_vector_active_subgraph_composition_pressure/graph_session_materialized_unresolved_current_filter/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.626 ms (quick) | Active unresolved-current set keeps full quality and cuts session candidates from 552 to 228. |
| `graph_vector_active_subgraph_composition_pressure/graph_session_provenance_expand_2hop_k16/9k_q64_c76_covbp10000_curbp10000_precbp10000` | 1.663 ms (quick) | Full-quality provenance reference over current roots; similar latency to materialized unresolved-current on this topology. |
| `graph_vector_active_subgraph_composition_pressure/graph_session_unresolved_provenance_expand_2hop_k16/9k_q64_c4_covbp3750_curbp3750_precbp4042` | 662.2 µs (quick) | Strict unresolved-root provenance is a negative result: very small root set, fast, but only partial coverage/precision. |
| `graph_vector_active_subgraph_composition_pressure/graph_scope_materialized_current_filter/9k_q64_c138_covbp10000_curbp10000_precbp10000` | 1.110 ms (quick) | Scope-local current baseline. |
| `graph_vector_active_subgraph_composition_pressure/graph_scope_materialized_unresolved_current_filter/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 497.3 µs (quick) | Scope-local active unresolved-current set keeps full quality and cuts candidates from 138 to 57. |
| `graph_vector_active_subgraph_composition_pressure/graph_scope_provenance_expand_2hop_k16/9k_q64_c19_covbp10000_curbp10000_precbp10000` | 611.4 µs (quick) | Full-quality scope provenance reference. |
| `graph_vector_active_subgraph_composition_pressure/graph_scope_unresolved_provenance_expand_2hop_k16/9k_q64_c1_covbp3750_curbp3750_precbp3750` | 179.4 µs (quick) | Strict scope unresolved-root provenance is also partial recall; active-set filtering needs a full-recall fallback. |
| `graph_vector_active_subgraph_composition_pressure/topic_filter/9k_q64_c152_covbp10000_curbp10000_precbp10000` | 1.095 ms (quick) | Metadata hard-topic reference. |

Active-subgraph fallback rows test that follow-up directly: run strict
unresolved-provenance first, then fill missing current facts from the
materialized unresolved-current active set. The fallback restores full quality,
but it is slower than scoring the maintained active set directly on this
fixture because it pays both the narrow provenance pass and the broad fallback
scoring pass:

| Bench | 9k/10k scale | Notes |
|---|---:|---|
| `graph_vector_active_subgraph_fallback_pressure/graph_session_materialized_unresolved_current_filter/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.673 ms (quick) | Full-quality maintained active-set baseline. |
| `graph_vector_active_subgraph_fallback_pressure/graph_session_unresolved_provenance_expand_2hop_k16/9k_q64_c4_covbp3750_curbp3750_precbp4042` | 664.1 µs (quick) | Fast strict provenance pass, still partial recall. |
| `graph_vector_active_subgraph_fallback_pressure/graph_session_unresolved_provenance_fallback_2hop_k16/9k_q64_c232_covbp10000_curbp10000_precbp10000` | 2.443 ms (quick) | Full-quality fallback, but slower than scoring the maintained active set directly. |
| `graph_vector_active_subgraph_fallback_pressure/graph_scope_materialized_unresolved_current_filter/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 523.6 µs (quick) | Scope-local maintained active-set baseline. |
| `graph_vector_active_subgraph_fallback_pressure/graph_scope_unresolved_provenance_expand_2hop_k16/9k_q64_c1_covbp3750_curbp3750_precbp3750` | 184.4 µs (quick) | Fast strict scope provenance, still partial recall. |
| `graph_vector_active_subgraph_fallback_pressure/graph_scope_unresolved_provenance_fallback_2hop_k16/9k_q64_c58_covbp10000_curbp10000_precbp10000` | 796.5 µs (quick) | Scope fallback restores full quality, but remains slower than the active-set baseline. |
| `graph_vector_active_subgraph_fallback_pressure/topic_filter/9k_q64_c152_covbp10000_curbp10000_precbp10000` | 1.131 ms (quick) | Metadata hard-topic reference. |

Active-hint rows keep the same noisy sparse multi-hop contradicted fixture, but
add graph-authored `RECENT_IN` windows and direct `DEPENDS_ON` edges. This
models a broad session query that can derive a narrower active subgraph from
task memory topology before exact vector scoring:

| Bench | 9k/10k scale | Notes |
|---|---:|---|
| `graph_vector_active_hint_pressure/graph_session_materialized_unresolved_current_filter/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.619 ms (quick) | Broad session active-set baseline. |
| `graph_vector_active_hint_pressure/graph_session_recent_active_filter/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 498.1 µs (quick) | Graph-authored recency window narrows the session to topic-sized active candidates while preserving full quality. |
| `graph_vector_active_hint_pressure/graph_session_dependency_active_filter/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 73.25 µs (quick) | Direct dependency edges produce one unresolved current candidate per fact; strongest full-quality graph/vector row so far on this fixture. |
| `graph_vector_active_hint_pressure/graph_session_provenance_expand_2hop_k16/9k_q64_c76_covbp10000_curbp10000_precbp10000` | 1.798 ms (quick) | Full-quality provenance reference is slower than direct active hints here. |
| `graph_vector_active_hint_pressure/topic_filter/9k_q64_c152_covbp10000_curbp10000_precbp10000` | 1.099 ms (quick) | Metadata hard-topic reference. |

Batched active-hint candidate-scoring rows compare repeated scoring, generic
`score_vector_nodes_batch_checked`, canonical `VectorCandidateSet` batch
scoring, sorted candidate-set algebra against the maintained unresolved-current
set, and graph-neighbor batch scoring. This isolates the API-boundary impact
from the candidate producer:

| Bench | 9k/10k scale | Notes |
|---|---:|---|
| `graph_vector_active_hint_batch_pressure/graph_session_materialized_unresolved_current_filter_repeated_score/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.666 ms (quick) | Existing repeated per-query candidate scoring over the broad active set. |
| `graph_vector_active_hint_batch_pressure/graph_session_materialized_unresolved_current_filter_batch_score/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.659 ms (quick) | Generic batch scorer is effectively neutral for broad candidate sets. |
| `graph_vector_active_hint_batch_pressure/graph_session_materialized_unresolved_current_filter_candidate_set_batch_score/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.659 ms (quick) | Canonical candidate-set batch scorer skips the second normalization pass and is modestly faster on broad sets. |
| `graph_vector_active_hint_batch_pressure/graph_session_materialized_unresolved_current_filter_candidate_set_algebra_batch_score/9k_q64_c228_covbp10000_curbp10000_precbp10000` | 1.511 ms (quick) | Sorted intersection of graph-session output with the maintained unresolved-current candidate set; broad-set algebra beats the HashSet-filtered candidate path on this fixture. |
| `graph_vector_active_hint_batch_pressure/graph_session_recent_active_filter_repeated_score/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 514.9 µs (quick) | Existing repeated scoring over recency-window candidates. |
| `graph_vector_active_hint_batch_pressure/graph_session_recent_active_filter_batch_score/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 514.9 µs (quick) | Generic batch scorer is effectively neutral for medium candidate sets and preserves full quality. |
| `graph_vector_active_hint_batch_pressure/graph_session_recent_active_filter_candidate_set_batch_score/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 512.5 µs (quick) | Canonical candidate-set batch scorer is only slightly faster than generic batch at this candidate width. |
| `graph_vector_active_hint_batch_pressure/graph_session_recent_active_filter_candidate_set_algebra_batch_score/9k_q64_c57_covbp10000_curbp10000_precbp10000` | 512.4 µs (quick) | Adaptive sorted intersection is now effectively neutral with the maintained recency candidate path at medium width. |
| `graph_vector_active_hint_batch_pressure/graph_session_dependency_active_filter_repeated_score/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 74.09 µs (quick) | Existing repeated scoring over direct dependency candidates. |
| `graph_vector_active_hint_batch_pressure/graph_session_dependency_active_filter_batch_score/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 73.98 µs (quick) | Generic batch scorer remains effectively neutral for tiny candidate sets. |
| `graph_vector_active_hint_batch_pressure/graph_session_dependency_active_filter_candidate_set_batch_score/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 74.68 µs (quick) | Canonical candidate-set batch scorer is neutral on tiny dependency sets. |
| `graph_vector_active_hint_batch_pressure/graph_session_dependency_active_filter_candidate_set_algebra_batch_score/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 74.06 µs (quick) | Adaptive intersection removes the prior large-active-set scan, making tiny dependency algebra neutral instead of the old ~126 µs failure mode. |
| `graph_vector_active_hint_batch_pressure/graph_session_dependency_active_filter_neighbor_score/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 73.58 µs (quick) | Production neighbor scorer derives dependency candidates from the anchor's `DEPENDS_ON` edges; the direct candidate-set scorer removes the extra normalization pass. |
| `graph_vector_active_hint_batch_pressure/graph_session_dependency_active_filter_neighbor_batch_score/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 73.25 µs (quick) | Batched production neighbor scorer remains the fastest dependency active-hint row on this fixture, while preserving full quality. |

ANN rerank rows use the same active-hint fixture, but start from ANN/search-hit
output. They convert wide ANN hits into `VectorCandidateSet`s, optionally
compose them with graph-maintained unresolved-current or dependency candidates,
then exact-score the resulting sets in one batch:

| Bench | 9k/10k scale | Notes |
|---|---:|---|
| `graph_vector_ann_rerank_pressure/ann_wide_hit_set_batch_rerank/9k_q64_c16_covbp1562_curbp1562_precbp9765` | 621.0 µs (quick) | Wide ANN hits stay high precision but low coverage; exact rerank of the ANN hit set cannot recover facts missing from the approximate candidate producer. |
| `graph_vector_ann_rerank_pressure/ann_wide_active_intersection_batch_rerank/9k_q64_c1_covbp644_curbp644_precbp644` | 524.6 µs (quick) | Adaptive intersection trims the active-set composition cost, but recall still collapses because the ANN seed rarely contains the active fact nodes. |
| `graph_vector_ann_rerank_pressure/ann_wide_dependency_union_batch_rerank/9k_q64_c23_covbp10000_curbp10000_precbp10000` | 694.4 µs (quick) | Unioning ANN hits with direct dependency candidates restores full quality, but the ANN search dominates latency and is roughly 10x slower than the graph-only dependency row. |
| `graph_vector_ann_rerank_pressure/graph_dependency_candidate_set_batch/9k_q64_c8_covbp10000_curbp10000_precbp10000` | 70.07 µs (quick) | Direct graph dependency candidates remain the best shape for tiny active hints; adding ANN output is unnecessary when graph topology already supplies one candidate per fact. |

Broad graph-gate rows use the same active-hint fixture but deliberately start
from the expensive session-level graph candidate set. They compare direct
session scoring, direct unresolved-current session scoring, ANN-only hits,
ANN-intersection gates, and ANN-union fallbacks:

| Bench | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_ann_broad_graph_gate_pressure/graph_session_candidate_set_batch/...covbp10000_curbp10000_precbp10000` | 343.2 µs (`c124`) | 3.827 ms (`c608`) | Broad session exact-score reference; full quality but expensive at 10k. |
| `graph_vector_ann_broad_graph_gate_pressure/graph_session_unresolved_candidate_set_batch/...covbp10000_curbp10000_precbp10000` | 217.5 µs (`c74`) | 1.505 ms (`c228`) | Maintained unresolved-current graph state cuts exact scoring while preserving full quality. |
| `graph_vector_ann_broad_graph_gate_pressure/ann_wide_hit_set_batch_rerank/...` | 262.8 µs (`c16`, `covbp5000`) | 639.4 µs (`c16`, `covbp1562`) | ANN-only hits are cheaper at 10k but miss most graph-memory facts. |
| `graph_vector_ann_broad_graph_gate_pressure/ann_wide_session_intersection_batch_rerank/...` | 280.9 µs (`c16`, `covbp5000`) | 783.5 µs (`c15`, `covbp1562`) | Intersecting 16 ANN hits with the broad session set trims exact scoring but does not improve recall. |
| `graph_vector_ann_broad_graph_gate_pressure/ann_broad_session_intersection_batch_rerank/...` | 538.2 µs (`c34`, full quality) | 1.442 ms (`c62`, `covbp5019`) | A 64-hit ANN gate recovers 1k quality but only reaches half coverage at 10k, roughly tied with direct unresolved-current scoring. |
| `graph_vector_ann_broad_graph_gate_pressure/ann_wide_session_union_batch_rerank/...` | 555.9 µs (`c124`) | 4.006 ms (`c608`) | ANN+session union preserves quality but adds ANN overhead without reducing the broad graph set. |
| `graph_vector_ann_broad_graph_gate_pressure/ann_broad_session_union_batch_rerank/...` | 847.6 µs (`c154`) | 4.466 ms (`c609`) | Wider ANN union is strictly slower than direct session scoring on this fixture. |

Interpretation: broad graph candidates are expensive enough that a gate would
matter, but ANN hit sets still do not contain enough graph-memory facts. The
best native shape remains graph-maintained unresolved-current state or stronger
graph-derived active hints; ANN should not be promoted as a broad-session
candidate gate without a different candidate producer or a much stronger recall
profile.

ANN+partial-graph fallback rows use the query-filter topology where label
propagation is a compact but partial graph-derived candidate producer. They
compare ANN alone, label propagation alone, their candidate-set union, and the
full-quality graph-scope reference:

| Bench | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_ann_graph_fallback_pressure/graph_scope_candidate_set_batch/...covbp10000_curbp10000_precbp10000` | 110.4 µs (`c32`) | 1.116 ms (`c152`) | Full-quality graph-scope reference using canonical candidate-set batch scoring. |
| `graph_vector_ann_graph_fallback_pressure/label_propagation_candidate_set_batch/...` | 54.89 µs (`c17`, `covbp7661`, `precbp8830`) | 117.8 µs (`c16`, `covbp7578`, `precbp8789`) | Label propagation is fast and compact but partial recall. |
| `graph_vector_ann_graph_fallback_pressure/ann_wide_hit_set_batch_rerank/...` | 265.5 µs (`c16`, `covbp5000`, `precbp10000`) | 624.9 µs (`c16`, `covbp1562`, `precbp9765`) | ANN alone is high precision but too low coverage to repair missing graph facts. |
| `graph_vector_ann_graph_fallback_pressure/ann_wide_label_union_batch_rerank/...` | 303.1 µs (`c28`, `covbp9959`, `precbp10000`) | 730.1 µs (`c30`, `covbp7910`, `precbp9980`) | ANN+label union nearly closes 1k coverage but barely improves the 10k label row while adding substantial ANN latency. |

Interpretation: ANN can help a small partial graph partition at 1k, but this
synthetic 10k fixture is still candidate-producer limited. A cheap graph-scope
candidate set remains the full-quality path; ANN fallback needs a different
workload where graph candidates are broad enough to be expensive but not
already quality-complete.

Adaptive provenance rows use the noisy sparse multi-hop topology and a
benchmark-only quality oracle. The adaptive row scores provenance roots once,
then tries k1 one-hop, k1 two-hop, k4 two-hop, k8 two-hop, and k16 two-hop
until full quality is reached or the ladder is exhausted:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_adaptive_provenance_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 376.3 µs (`c113`) | 4.529 ms (`c552`) | Full-quality broad session-current baseline on the adaptive run. |
| `graph_vector_adaptive_provenance_pressure/graph_session_provenance_expand_2hop_k16/...covbp10000_curbp10000_precbp10000` | 232.5 µs (`c15`) | 2.336 ms (`c76`) | Fixed wide two-hop session reference. |
| `graph_vector_adaptive_provenance_pressure/graph_session_provenance_adaptive_quality/...covbp10000_curbp10000_precbp10000` | 200.7 µs (`c15`) | 2.402 ms (`c76`) | Stops early at 1k, but at 10k pays the staged probes before reaching the same full-quality plan. |
| `graph_vector_adaptive_provenance_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 116.8 µs (`c29`) | 1.403 ms (`c138`) | Full-quality scope-current baseline on the adaptive run. |
| `graph_vector_adaptive_provenance_pressure/graph_scope_provenance_expand_2hop_k16/...covbp10000_curbp10000_precbp10000` | 77.61 µs (`c4`) | 802.3 µs (`c19`) | Fixed wide two-hop scope reference. |
| `graph_vector_adaptive_provenance_pressure/graph_scope_provenance_adaptive_quality/...covbp10000_curbp10000_precbp10000` | 98.00 µs (`c4`) | 875.9 µs (`c19`) | Oracle-style staged probing is slower than fixed k16 on the scope path. |
| `graph_vector_adaptive_provenance_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 114.8 µs (`c32`) | 1.195 ms (`c152`) | Metadata hard-topic lower-bound reference for comparison. |

Negative-evidence rows use a graph-authored `CONTRADICTS` edge to mark duplicate
current facts as resolved elsewhere, leaving one unresolved current node per
fact. This measures graph-side candidate pruning before exact vector scoring:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_negative_evidence_pressure/graph_session_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 228.2 µs (`c70`) | 2.424 ms (`c356`) | Baseline broad session-current scoring still sees contradicted duplicate current facts. |
| `graph_vector_negative_evidence_pressure/graph_session_unresolved_current_filter/...covbp10000_curbp10000_precbp10000` | 160.6 µs (`c31`) | 976.1 µs (`c32`) | Dynamic graph-derived unresolved-current pruning keeps full quality while cutting the 10k session row by ~2.5x. |
| `graph_vector_negative_evidence_pressure/graph_session_materialized_unresolved_current_filter/...covbp10000_curbp10000_precbp10000` | 122.0 µs (`c31`) | 556.6 µs (`c32`) | Maintaining the unresolved-current set keeps the same full-quality candidate shape while avoiding per-candidate edge scans. |
| `graph_vector_negative_evidence_pressure/graph_scope_materialized_current_filter/...covbp10000_curbp10000_precbp10000` | 74.29 µs (`c18`) | 775.4 µs (`c89`) | Topic-scope current baseline with contradicted duplicates still present. |
| `graph_vector_negative_evidence_pressure/graph_scope_unresolved_current_filter/...covbp10000_curbp10000_precbp10000` | 53.33 µs (`c8`) | 289.8 µs (`c8`) | Dynamic unresolved-current pruning keeps the strongest exact-scored graph candidate shape. |
| `graph_vector_negative_evidence_pressure/graph_scope_materialized_unresolved_current_filter/...covbp10000_curbp10000_precbp10000` | 42.94 µs (`c8`) | 173.9 µs (`c8`) | Materialized unresolved-current scope pruning is the fastest full-quality graph row in this matrix. |
| `graph_vector_negative_evidence_pressure/topic_filter/...covbp10000_curbp10000_precbp10000` | 104.7 µs (`c32`) | 1.105 ms (`c152`) | Metadata hard-topic reference remains slower than unresolved graph pruning at 10k. |

Active-set maintenance rows reuse the negative-evidence fixture and time a
60-read / 40-write cycle. The dynamic row pays graph-edge checks on every read
and no maintained-state work on writes. The materialized row pays maintained
set membership on reads and a balanced 20-remove / 20-insert active-set update
cycle on writes:

| Strategy | 1k requested / 992 actual | 10k requested / 9,728 actual | Notes |
|---|---:|---:|---|
| `graph_vector_active_set_maintenance_pressure/dynamic_edge_checks_r60w40/...covbp10000_curbp10000_precbp10000` | 9.622 ms (`c31`) | 58.35 ms (`c32`) | Baseline 60/40 cycle using dynamic contradiction-edge checks for unresolved-current reads. |
| `graph_vector_active_set_maintenance_pressure/materialized_set_r60w40/...covbp10000_curbp10000_precbp10000` | 6.904 ms (`c31`) | 26.41 ms (`c32`) | Maintained active set keeps full quality and remains faster even after 40 balanced set updates per cycle. |
| `graph_vector_active_set_maintenance_pressure/materialized_set_maintenance_w40/...` | 454.1 ns (`active248`) | 396.8 ns (`active512`) | Isolated 40-update HashSet maintenance is negligible next to exact vector rerank cost on this fixture. |

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
