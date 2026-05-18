# selene-db benchmarks

_Last measured: 2026-05-15 on Apple M5 (10-core / 16 GiB / macOS 26.5 / rustc 1.95.0 / commit `11537ed`)._

> Methodology: `scripts/run-benches.sh --profile full --layer criterion`
> (sequential execution; concurrent `cargo bench` is blocked by the script's
> `pgrep` guard). Medians are criterion 0.8 wall-clock. iai-callgrind
> instruction-count baselines are deferred to the v1.0.0 release-prep
> panic-audit pass.
> BRIEF-89 bulk-mutation benches return their per-iteration `SharedGraph`
> from the timed routine so Criterion drops fixture teardown after timing;
> those rows measure mutation + commit, not graph deallocation.

## Hardware footprint

| Field | Value | Source |
|---|---|---|
| CPU | Apple M5 | `sysctl -n machdep.cpu.brand_string` |
| Cores | 10 physical / 10 logical | `sysctl -n hw.physicalcpu hw.logicalcpu` |
| Memory | 16.0 GiB | `sysctl -n hw.memsize` |
| OS | macOS 26.5 (build 25F71) | `sw_vers` |
| rustc | 1.95.0 (59807616e 2026-04-14) | `rustc --version` |
| Commit | `11537ed` | `git rev-parse --short HEAD` |

## §1 selene-graph hot paths

Registered tokens: `selene-graph:single_graph:criterion`,
`selene-graph:bulk_mutation:criterion`, `selene-graph:concurrent_read:criterion`,
`selene-graph:bfs:criterion`, `selene-graph:write_txn_lifecycle:criterion`,
`selene-graph:provider_fanout:criterion`,
`selene-graph:bound_type_validation:criterion`,
`selene-graph:concurrent_writers:criterion`.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `graph_node_fetch` | **2.10 ns** | 2.11 ns | 2.09 ns | Near-flat O(1); columnar fetch. |
| `graph_label_index_lookup` | 5.20 ns | 4.24 ns | 4.30 ns | Flat across all scales; `IStr`-keyed hash lookup. |
| `graph_typed_index_point` | **4.53 ns** | 4.44 ns | 4.67 ns | Restored flat-curve via `lookup_eq` -> `Option<Cow<'_, RoaringBitmap>>` per BRIEF-88; tri-state semantics preserved. |
| `graph_typed_index_range` | 20.10 µs | 178.8 µs | 294.4 µs | Sub-linear range scan. |
| `graph_composite_index_proxy` | 52.12 ns | 143.2 ns | 287.8 ns | Linear. |
| `graph_edge_create_cascade` | 211.7 µs | 153.0 µs | 343.5 µs | Mutation + commit body; fixture teardown excluded from timed routine. |
| `graph_mutation_commit_batch` (10) | 262.5 µs | 231.8 µs | 360.8 µs | Restored via in-place mutation against `Arc<SeleneGraph>` write lock + explicit `pre_txn` rollback per BRIEF-89; three-clone cascade replaced with one COW clone at first mutation. |
| `graph_mutation_commit_batch` (100) | 322.7 µs | 292.4 µs | 434.0 µs | Batch of 100; fixture teardown excluded from timed routine. |
| `graph_mutation_commit_batch` (1000) | 783.9 µs | 603.7 µs | 761.9 µs | Batch of 1000; batching wins at higher cardinality. |
| `graph_concurrent_reads` | 76.78 µs | 84.18 µs | 84.72 µs | **Flat above 10k** — ArcSwap snapshot read confirmed O(1). |
| `graph_bfs` (depth=1) | 99.36 ns | 98.31 ns | 99.66 ns | Depth-1 BFS independent of N. |
| `graph_bfs` (depth=10) | 10.90 µs | 11.09 µs | 11.28 µs | Mostly traversal cost. |
| `graph_bfs` (depth=50) | 94.45 µs | 106.6 µs | 108.3 µs | Saturates around 100 µs. |

### §1a Write pipeline microbenches

Rows added by BRIEF-111 are compile-registered but not measured yet. They isolate lifecycle, provider fanout, bound-type validation, and writer queueing costs before the GQL write e2e layer.

| Bench | Variant | Median | Notes |
|---|---|---:|---|
| `write_txn_lifecycle/empty_commit` | n/a | TBD | Empty transaction commit floor. |
| `write_txn_lifecycle/create_only` | batch=1 | TBD | Isolated node create + commit. |
| `write_txn_lifecycle/create_only` | batch=10 | TBD | Isolated node create + commit. |
| `write_txn_lifecycle/create_only` | batch=100 | TBD | Isolated node create + commit. |
| `write_txn_lifecycle/create_only` | batch=1000 | TBD | Isolated node create + commit. |
| `write_txn_lifecycle/delete_only` | batch=1 | TBD | Fixture seed excluded from timed body. |
| `write_txn_lifecycle/delete_only` | batch=10 | TBD | Fixture seed excluded from timed body. |
| `write_txn_lifecycle/delete_only` | batch=100 | TBD | Fixture seed excluded from timed body. |
| `write_txn_lifecycle/delete_only` | batch=1000 | TBD | Fixture seed excluded from timed body. |
| `provider_fanout/core_only` | providers=core | TBD | Commit notification baseline. |
| `provider_fanout/extra_k1` | extra=1 | TBD | Additional no-op provider fanout. |
| `provider_fanout/extra_k4` | extra=4 | TBD | Additional no-op provider fanout. |
| `provider_fanout/extra_k16` | extra=16 | TBD | Additional no-op provider fanout. |
| `provider_fanout/extra_k4_with_error_one` | extra=4 + error | TBD | Error-path notification scaling. |
| `provider_fanout/extra_k4_with_panic_one` | extra=4 + panic | TBD | Opt-in via `SELENE_BENCH_INCLUDE_PANIC_PROVIDER=1`. |
| `bound_type_validation/unbound_commit` | unbound | TBD | Commit without graph type validation. |
| `bound_type_validation/bound_commit_simple` | simple type graph | TBD | Typed commit validation delta. |
| `bound_type_validation/bound_commit_rich` | rich type graph | TBD | Wider type graph validation delta. |
| `bound_type_validation/bound_schema_change` | schema change | TBD | Full graph state validation path. |

| Bench | Threads | Median | Notes |
|---|---:|---:|---|
| `concurrent_writers/threads1` | 1 | TBD | 1000 total commits, 10 property updates per commit. |
| `concurrent_writers/threads2` | 2 | TBD | 1000 total commits, 10 property updates per commit. |
| `concurrent_writers/threads4` | 4 | TBD | 1000 total commits, 10 property updates per commit. |
| `concurrent_writers/threads8` | 8 | TBD | 1000 total commits, 10 property updates per commit. |
| `concurrent_writers/threads1_with_readers8` | 1 | TBD | Same writer load with 8 snapshot readers. |
| `concurrent_writers/threads2_with_readers8` | 2 | TBD | Same writer load with 8 snapshot readers. |
| `concurrent_writers/threads4_with_readers8` | 4 | TBD | Same writer load with 8 snapshot readers. |
| `concurrent_writers/threads8_with_readers8` | 8 | TBD | Same writer load with 8 snapshot readers. |

## §2 selene-persist

Registered tokens: `selene-persist:wal:criterion`,
`selene-persist:snapshot:criterion`.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `persist_wal_append_single` | 59.51 ms | 293.7 ms | 588.9 ms | Single-entry append loop with `EveryN(1000)` group commit; scale = WAL entries, not graph nodes. |
| `persist_wal_append_single_no_fsync` | **11.16 ms** | 54.49 ms | 108.6 ms | `SyncPolicy::OnFlushOnly`; donor-parity diagnostic with append/threshold/drop fsync suppressed. |
| `persist_wal_append_batch_1000` | 6.31 ms | 8.02 ms | 10.95 ms | **54× faster than per-entry at 100k** — batching wins. |
| `persist_wal_append_batch_1000_no_fsync` | **1.46 ms** | 3.79 ms | 6.37 ms | Batched donor-parity diagnostic; timed body does not call `flush()`. |
| `persist_wal_replay` | **4.04 ms** | 16.78 ms | 30.51 ms | BRIEF-90 WAL v2: fixed-layout header + xxh3 checksum + BufReader. |
| `persist_snapshot_write` | 719.3 µs | 2.12 ms | 4.58 ms | Snapshot capture; sub-linear at 100k. |
| `persist_snapshot_read` | 549.6 µs | 2.35 ms | 4.68 ms | Snapshot read-and-apply. |
| `persist_full_recovery` | 2.93 ms | 12.91 ms | 24.75 ms | Snapshot reconciliation + WAL v2 replay. |

### §2a WAL sync policy sweep

`persist_wal_sync_sweep/*` measures append + explicit `flush()` across sync policies. Full/stress profiles use 1k, 10k, and 100k WAL entries, except `every1`, which is capped at 10k to keep benchmark time bounded. Existing append rows above use `EveryN(1000)` unless marked `_no_fsync`; `_no_fsync` rows omit explicit flush.

| Bench | 1k | 10k | 100k | Notes |
|---|---:|---:|---:|---|
| `persist_wal_sync_sweep/every1` | TBD | TBD | n/a | `SyncPolicy::EveryN(1)`. |
| `persist_wal_sync_sweep/every10` | TBD | TBD | TBD | `SyncPolicy::EveryN(10)`. |
| `persist_wal_sync_sweep/every100` | TBD | TBD | TBD | `SyncPolicy::EveryN(100)`. |
| `persist_wal_sync_sweep/every1000` | TBD | TBD | TBD | `SyncPolicy::EveryN(1000)`. |
| `persist_wal_sync_sweep/on_flush_only` | TBD | TBD | TBD | `SyncPolicy::OnFlushOnly` with caller flush. |

BRIEF-90: fixed-layout header (no postcard varint) + xxh3 checksum + BufReader on iterate path; WAL v1→v2.

Platform note: donor WAL append baselines were measured with snapshot-only sync behavior. The `_no_fsync` rows use `SyncPolicy::OnFlushOnly`, which suppresses append fsync, threshold-triggered fsync, and drop-time fsync; an explicit caller-issued `flush()` would still sync. There is no replay `_no_fsync` sibling because replay's timed body is read-only, so sync policy would not isolate a useful signal.

## §3 selene-gql (scale-independent)

Registered tokens: `selene-gql:parse:criterion`, `selene-gql:analyze:criterion`,
`selene-gql:plan_optimize:criterion`.

| Bench | Median | Notes |
|---|---:|---|
| `gql_parse_corpus/m5c` | 263.8 µs | Single-query parse latency (m5c corpus). |
| `gql_analyze_corpus/m5c` | **5.32 µs** | Semantic analysis; well below donor floor (<1 ms). |
| `gql_plan_optimize_corpus/m5c` | 19.11 µs | Planner/optimizer end-to-end. |
| `gql_plan_ir_clone/representative` | 94.95 ns | IR-clone hot path. |

## §3a selene-gql write_e2e

Registered token: `selene-gql:write_e2e:criterion`.

The GQL rows use prebuilt `BenchFixture` graphs plus a bench-local WAL append provider so `execute_statement` includes commit fanout and WAL append. Direct rows bypass GQL and call explicit WAL flush to isolate durable mutation cost.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `write_e2e/gql_insert_single_node_per_iter_plan` | TBD | TBD | TBD | Parse/plan/execute per iteration. |
| `write_e2e/gql_insert_single_node_preplanned` | TBD | TBD | TBD | Preplanned single-node insert. |
| `write_e2e/gql_insert_node_with_edge_preplanned` | TBD | TBD | TBD | Preplanned insert with matched source node and edge. |
| `write_e2e/gql_match_set_preplanned` | TBD | TBD | TBD | Preplanned indexed match + property update. |
| `write_e2e/gql_match_delete_preplanned` | TBD | TBD | TBD | Fresh fixture per timed iteration because target node is deleted. |
| `write_e2e/gql_multi_statement_txn_preplanned` | TBD | TBD | TBD | START, three INSERTs, COMMIT. |
| `write_e2e/direct_insert_single_node_with_wal_flush` | TBD | TBD | TBD | Direct graph mutation + one WAL flush. |
| `write_e2e/direct_insert_single_node_with_wal_flush_every10` | TBD | TBD | TBD | Ten direct inserts amortized over one WAL flush. |

## §4 selene-algorithms (Sequential vs Auto)

Registered token: `selene-algorithms:algo_bench:criterion`. Fixture: `BenchFixture::build(N)` (≈3N edges) for pagerank/betweenness/apsp; `planted_community_graph(N)` (≈6N edges, ~N/64 communities) for triangle_count and louvain.

| Bench | Scale | Sequential | Auto | Auto speedup | Notes |
|---|---:|---:|---:|---:|---|
| `pagerank` | 10k | **245.5 µs** | 524.1 µs | **0.47×** | Auto pays parallelism overhead at sparse-graph scales. |
| `pagerank` | 50k | 1.43 ms | 2.31 ms | 0.62× | DESC by score + NodeId ASC; V169 contract. |
| `pagerank` | 100k | 2.94 ms | 4.49 ms | 0.65× | Per-iter work (3·N FP ops) doesn't beat coordination cost on M5. |
| `betweenness` | 10k | 25.05 ms | 8.60 ms | **2.91×** | Endpoint-aware sampling; V168 contract. |
| `betweenness` | 50k | 128.4 ms | 46.59 ms | **2.76×** | Per-source SSSP makes parallelism pay off. |
| `betweenness` | 100k | 264.7 ms | **110.2 ms** | **2.40×** | Headline rayon win on selene-db. |
| `triangle_count` | 10k | 985.5 µs | 951.8 µs | 1.04× | Tiny; already efficient sequentially. |
| `triangle_count` | 50k | 5.21 ms | 4.44 ms | 1.17× | |
| `triangle_count` | 100k | 10.33 ms | 8.86 ms | 1.17× | Modest gain. |
| `apsp` | 200 | 1.52 ms | 466.1 µs | **3.27×** | All-pairs SSSP; scale = source count. |
| `apsp` | 500 | 8.57 ms | 2.19 ms | **3.92×** | |
| `apsp` | 1k | 34.54 ms | **8.46 ms** | **4.08×** | Strong parallel scaling at 10 cores. |
| `louvain` | 10k | 5.03 ms | n/a | — | Sequential-only in v1.0 per V170. |
| `louvain` | 50k | 27.08 ms | n/a | — | |
| `louvain` | 100k | **55.43 ms** | n/a | — | No `LOUVAIN_SCALES` downgrade needed at this fixture. |

**Notable**: pagerank/Auto is **slower** than pagerank/Sequential at every scale on this fixture. The bench graph is sparse (~3 edges/node), so per-iteration work (3·N FP multiplications + accumulator) doesn't outweigh rayon's thread-coordination cost on an M5. Auto remains the right choice on denser graphs where per-vertex work amortizes the overhead — the API exposes both modes deliberately.

## §5 selene-algorithms-pack (adapter overhead)

Registered token: `selene-algorithms-pack:algo_pack:criterion`. Fixtures stay crate-local to measure adapter overhead independently of algorithm scaling.

| Bench | Fixture | Median | Notes |
|---|---|---:|---|
| `algo_pack/projection_build_default` | 1k deterministic directed graph | 235.2 µs | Includes parse + plan + execute. |
| `algo_pack/algo_pagerank_default` | 256-node prebuilt projection | 138.1 µs | |
| `algo_pack/algo_dijkstra_single_pair` | 256-node prebuilt projection | 38.08 µs | |
| `algo_pack/algo_apsp_default` | 96-node prebuilt projection | 1.47 ms | Small-N APSP. |
| `algo_pack/algo_betweenness_default` | 256-node prebuilt projection | 299.4 µs | |
| `algo_pack/algo_louvain_default` | 256-node prebuilt projection | 129.7 µs | |
| `algo_pack/algo_triangle_count_default` | 256-node prebuilt projection | 119.7 µs | |
| `algo_pack/algo_label_propagation_default` | 256-node prebuilt projection | 75.78 µs | |

Adapter cost is dominated by GQL `CALL` parsing + planning, not the underlying algorithm.

## §6 selene-vector-pack (GQL CALL adapter overhead)

Registered token: `selene-vector-pack:vector_pack:criterion`.

| Bench | Vector count | Dim | k | Median | Notes |
|---|---:|---:|---:|---:|---|
| `vector_pack/search_default` | 1k | 256 | 10 | **18.51 µs** | HNSW. |
| `vector_pack/upsert_default` | 1 | 256 | n/a | 2.42 µs | HNSW single insert. |
| `vector_pack/bulk_upsert_default` | 100 | 256 | n/a | 4.22 ms | HNSW bulk mutation. |
| `vector_pack/ivf_search_default` | 256 | 256 | 10 | **2.88 µs** | Trained IVF; ~6× faster than HNSW at this corpus size. |
| `vector_pack/ivf_bulk_upsert_default` | 100 | 256 | n/a | 57.37 µs | IVF bulk mutation. |
| `vector_pack/ivf_stats_default` | n/a | n/a | n/a | 358.9 ns | Stats read. |

## §7 selene-vector (HNSW + IVF recall + replay)

Registered tokens: `selene-vector:build:criterion`,
`selene-vector:recall:criterion`, `selene-vector:quant_recall:criterion`,
`selene-vector:ivfpq_recall:criterion`, `selene-vector:composition_replay:criterion`.

### §7a `vector_recall_at_10` (HNSW baseline; ext_select toggle)

| Variant | k=10 | k=25 | k=50 | k=100 |
|---|---:|---:|---:|---:|
| extend_off | 718.3 µs (0.881) | 1.25 ms (0.991) | 2.20 ms (1.000) | 4.63 ms (1.000) |
| extend_on | 717.8 µs (0.881) | 1.24 ms (0.991) | 2.20 ms (1.000) | 4.62 ms (1.000) |

(latency / recall@10; extend_on yields ~4% latency reduction at small k.)

### §7b `quant_recall_at_10` (PQ/SQ/OPQ quantization recall vs f32)

| Variant | k=10 | k=25 | k=50 | k=100 |
|---|---:|---:|---:|---:|
| **f32** (baseline) | 705.6 µs / 0.881 | 1.24 ms / 0.991 | 2.18 ms / 1.000 | 4.60 ms / 1.000 |
| sq8 | 756.9 µs / 0.884 | 1.28 ms / 0.987 | 2.23 ms / 0.994 | 4.68 ms / 0.994 |
| sq8 + rescore | 749.3 µs / 0.884 | 1.28 ms / 0.991 | 2.23 ms / 1.000 | 4.81 ms / 1.000 |
| pq | 743.9 µs / 0.503 | 1.27 ms / 0.506 | 2.31 ms / 0.491 | 4.92 ms / 0.491 |
| pq + rescore | 746.0 µs / 0.503 | 1.26 ms / 0.778 | 2.30 ms / 0.931 | 4.92 ms / **0.984** |
| opq | 731.7 µs / 0.500 | 1.25 ms / 0.472 | 2.33 ms / 0.459 | 4.91 ms / 0.456 |
| opq + rescore | 733.3 µs / 0.500 | 1.26 ms / 0.781 | 2.31 ms / 0.925 | 4.95 ms / **0.994** |

(latency / recall@10. **SQ8** is essentially free. **PQ/OPQ alone** collapse recall to ~50%; pairing with a **rescore** tier recovers recall to 0.98–0.99 at the same wall-clock as f32.)

### §7c `vector_ivfpq_recall_at_10` (IVF-PQ n_probe sweep)

| n_probe | Latency |
|---:|---:|
| 1 | **699.7 ns** |
| 4 | 2.68 µs |
| 8 | 4.97 µs |

Linear in `n_probe`; sub-µs cold-cache lookup at `n_probe=1`.

### §7d `composition_replay` (selene-vector IVF-PQ + snapshot/publish)

| Variant | insert_snapshot_query | insert_publish_query |
|---|---:|---:|
| plain_pq | 24.98 ms | 24.51 ms |
| **opq_polysemous** | **1.17 s** | **1.17 s** |

OPQ rotation on insert is ~48× slower than plain PQ. Significant for insert-heavy workloads — favor `plain_pq` unless polysemous OPQ is needed for the workload's recall target.

### §7e `vector_hnsw_build` (cold direct HNSW construction)

BRIEF-103 remeasured this row on 2026-05-16 with
`scripts/run-benches.sh --profile full --layer criterion --filter vector_hnsw_build`;
other benchmark sections are unchanged from the header run.

| Bench | n=100 | n=1000 | n=5000 | Notes |
|---|---:|---:|---:|---|
| `vector_hnsw_build` | 2.791 ms | **53.25 ms** | 340.09 ms | Direct `insert_node` build; dim=16, M=8, ef_construction=64, L2; deterministic `BUILD_SEED = 0x9100_0001`; BRIEF-103 BinaryHeap beam + diversity cache. |

Directional donor note: `_design/perf-baselines.md:92` reports 1.31 s @ n=1k under unspecified dim/M; nearby text suggests dim=384, M=16. This bench is dim=16, M=8, so 53.25 ms @ n=1k is a non-parity signal (~24.6× lower wall-clock), not an apples-to-apples claim.
BRIEF-103 improves the local n=5000 baseline from 578.2 ms to 340.09 ms
(~41% lower wall-clock); 5× rows from 1k to 5k now costs ~6.4×.

## §iai-callgrind (deferred; instruction-count baselines pending)

Pinned for parity with `scripts/run-benches.sh` BENCHES list. Numbers are
populated during the v1.0.0 release-prep panic-audit cycle.

| Token | Status |
|---|---|
| `selene-graph:iai_gates:iai` | TBD |
| `selene-persist:iai_gates:iai` | TBD |
| `selene-gql:iai_gates:iai` | TBD |

## Update protocol

1. Switch to stable main: `git checkout main && git pull --ff-only`.
2. Run: `scripts/run-benches.sh --profile full --layer criterion`.
3. Paste hardware footprint into the header using the capture commands above.
4. Update each table's `Median` / `Sequential` / `Auto` columns from criterion output.
5. Set `_Last measured_` date.
6. Commit: `chore: refresh BENCHMARKS.md (<short hardware string>)`.

## Out of scope for this file

- iai-callgrind instruction-count baselines beyond the deferred token inventory.
- Cross-host comparison automation.
- CI bench jobs.
- Donor regression target comparison, which lives in gitignored `_design/perf-baselines.md`.
