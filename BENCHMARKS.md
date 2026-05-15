# selene-db benchmarks

_Last measured: 2026-05-15 on Apple M5 (10-core / 16 GiB / macOS 26.5 / rustc 1.95.0 / commit `2e56f23`)._

> Methodology: `scripts/run-benches.sh --profile full --layer criterion`
> (sequential execution; concurrent `cargo bench` is blocked by the script's
> `pgrep` guard). Medians are criterion 0.8 wall-clock. iai-callgrind
> instruction-count baselines are deferred to the v1.0.0 release-prep
> panic-audit pass.

## Hardware footprint

| Field | Value | Source |
|---|---|---|
| CPU | Apple M5 | `sysctl -n machdep.cpu.brand_string` |
| Cores | 10 physical / 10 logical | `sysctl -n hw.physicalcpu hw.logicalcpu` |
| Memory | 16.0 GiB | `sysctl -n hw.memsize` |
| OS | macOS 26.5 (build 25F71) | `sw_vers` |
| rustc | 1.95.0 (59807616e 2026-04-14) | `rustc --version` |
| Commit | `2e56f23` | `git rev-parse --short HEAD` |

## §1 selene-graph hot paths

Registered tokens: `selene-graph:single_graph:criterion`,
`selene-graph:bulk_mutation:criterion`, `selene-graph:concurrent_read:criterion`,
`selene-graph:bfs:criterion`.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `graph_node_fetch` | **2.00 ns** | 2.08 ns | 2.12 ns | Near-flat O(1); columnar fetch. |
| `graph_label_index_lookup` | 4.24 ns | 4.22 ns | 4.27 ns | Flat across all scales; `IStr`-keyed hash lookup. |
| `graph_typed_index_point` | 17.94 ns | 23.56 ns | 36.00 ns | Mild growth; `cloned_or_empty` + imbl SipHash13 collision-chain cost at 100k (see `project_hot_read_path_perf`). |
| `graph_typed_index_range` | 20.57 µs | 178.8 µs | 296.3 µs | Sub-linear range scan. |
| `graph_composite_index_proxy` | 62.33 ns | 155.7 ns | 309.4 ns | Linear. |
| `graph_edge_create_cascade` | 1.08 ms | 2.71 ms | 5.06 ms | Linear write amortization. |
| `graph_mutation_commit_batch` (10) | 1.10 ms | 2.81 ms | 4.84 ms | Batch of 10 mutations per commit. |
| `graph_mutation_commit_batch` (100) | 1.15 ms | 2.90 ms | 5.13 ms | Batch of 100. |
| `graph_mutation_commit_batch` (1000) | 1.54 ms | 3.47 ms | 5.06 ms | Batch of 1000; batching wins at higher cardinality. |
| `graph_concurrent_reads` | 75.27 µs | 82.49 µs | 82.51 µs | **Flat above 10k** — ArcSwap snapshot read confirmed O(1). |
| `graph_bfs` (depth=1) | 95.76 ns | 93.60 ns | 94.62 ns | Depth-1 BFS independent of N. |
| `graph_bfs` (depth=10) | 10.42 µs | 11.08 µs | 11.24 µs | Mostly traversal cost. |
| `graph_bfs` (depth=50) | 93.20 µs | 103.7 µs | 107.7 µs | Saturates around 100 µs. |

## §2 selene-persist

Registered tokens: `selene-persist:wal:criterion`,
`selene-persist:snapshot:criterion`.

| Bench | 10k | 50k | 100k | Notes |
|---|---:|---:|---:|---|
| `persist_wal_append_single` | 78.74 ms | 339.8 ms | 578.9 ms | Per-entry fsync; scale = WAL entries, not graph nodes. |
| `persist_wal_append_batch_1000` | 5.39 ms | 8.05 ms | 10.78 ms | **53× faster than per-entry at 100k** — batching wins. |
| `persist_wal_replay` | 50.74 ms | 275.6 ms | 550.7 ms | Linear in entry count. |
| `persist_snapshot_write` | 629.1 µs | 1.84 ms | 3.72 ms | Snapshot capture; sub-linear at 100k. |
| `persist_snapshot_read` | 557.0 µs | 2.39 ms | 4.58 ms | Snapshot read-and-apply. |
| `persist_full_recovery` | 26.32 ms | 136.6 ms | 279.2 ms | Snapshot reconciliation + WAL replay. |

## §3 selene-gql (scale-independent)

Registered tokens: `selene-gql:parse:criterion`, `selene-gql:analyze:criterion`,
`selene-gql:plan_optimize:criterion`.

| Bench | Median | Notes |
|---|---:|---|
| `gql_parse_corpus/m5c` | 283.4 µs | Single-query parse latency (m5c corpus). |
| `gql_analyze_corpus/m5c` | **5.51 µs** | Semantic analysis; well below donor floor (<1 ms). |
| `gql_plan_optimize_corpus/m5c` | 18.07 µs | Planner/optimizer end-to-end. |
| `gql_plan_ir_clone/representative` | 94.66 ns | IR-clone hot path. |

## §4 selene-algorithms (Sequential vs Auto)

Registered token: `selene-algorithms:algo_bench:criterion`. Fixture: `BenchFixture::build(N)` (≈3N edges) for pagerank/betweenness/apsp; `planted_community_graph(N)` (≈6N edges, ~N/64 communities) for triangle_count and louvain.

| Bench | Scale | Sequential | Auto | Auto speedup | Notes |
|---|---:|---:|---:|---:|---|
| `pagerank` | 10k | **247.2 µs** | 649.8 µs | **0.38×** | Auto pays parallelism overhead at sparse-graph scales. |
| `pagerank` | 50k | 1.42 ms | 2.89 ms | 0.49× | DESC by score + NodeId ASC; V169 contract. |
| `pagerank` | 100k | 2.89 ms | 5.95 ms | 0.49× | Per-iter work (3·N FP ops) doesn't beat coordination cost on M5. |
| `betweenness` | 10k | 24.82 ms | 8.60 ms | **2.89×** | Endpoint-aware sampling; V168 contract. |
| `betweenness` | 50k | 124.9 ms | 43.40 ms | **2.88×** | Per-source SSSP makes parallelism pay off. |
| `betweenness` | 100k | 251.2 ms | **100.0 ms** | **2.51×** | Headline rayon win on selene-db. |
| `triangle_count` | 10k | 940.4 µs | 951.1 µs | 0.99× | Tiny; already efficient sequentially. |
| `triangle_count` | 50k | 5.00 ms | 4.33 ms | 1.16× | |
| `triangle_count` | 100k | 10.68 ms | 9.17 ms | 1.16× | Modest gain. |
| `apsp` | 200 | 1.59 ms | 503.0 µs | **3.16×** | All-pairs SSSP; scale = source count. |
| `apsp` | 500 | 8.97 ms | 2.52 ms | **3.56×** | |
| `apsp` | 1k | 36.65 ms | **9.68 ms** | **3.78×** | Strong parallel scaling at 10 cores. |
| `louvain` | 10k | 5.24 ms | n/a | — | Sequential-only in v1.0 per V170. |
| `louvain` | 50k | 27.62 ms | n/a | — | |
| `louvain` | 100k | **57.14 ms** | n/a | — | No `LOUVAIN_SCALES` downgrade needed at this fixture. |

**Notable**: pagerank/Auto is **slower** than pagerank/Sequential at every scale on this fixture. The bench graph is sparse (~3 edges/node), so per-iteration work (3·N FP multiplications + accumulator) doesn't outweigh rayon's thread-coordination cost on an M5. Auto remains the right choice on denser graphs where per-vertex work amortizes the overhead — the API exposes both modes deliberately.

## §5 selene-algorithms-pack (adapter overhead)

Registered token: `selene-algorithms-pack:algo_pack:criterion`. Fixtures stay crate-local to measure adapter overhead independently of algorithm scaling.

| Bench | Fixture | Median | Notes |
|---|---|---:|---|
| `algo_pack/projection_build_default` | 1k deterministic directed graph | 257.2 µs | Includes parse + plan + execute. |
| `algo_pack/algo_pagerank_default` | 256-node prebuilt projection | 144.0 µs | |
| `algo_pack/algo_dijkstra_single_pair` | 256-node prebuilt projection | 42.32 µs | |
| `algo_pack/algo_apsp_default` | 96-node prebuilt projection | 1.60 ms | Small-N APSP. |
| `algo_pack/algo_betweenness_default` | 256-node prebuilt projection | 333.4 µs | |
| `algo_pack/algo_louvain_default` | 256-node prebuilt projection | 124.2 µs | |
| `algo_pack/algo_triangle_count_default` | 256-node prebuilt projection | 119.9 µs | |
| `algo_pack/algo_label_propagation_default` | 256-node prebuilt projection | 78.48 µs | |

Adapter cost is dominated by GQL `CALL` parsing + planning, not the underlying algorithm.

## §6 selene-vector-pack (GQL CALL adapter overhead)

Registered token: `selene-vector-pack:vector_pack:criterion`.

| Bench | Vector count | Dim | k | Median | Notes |
|---|---:|---:|---:|---:|---|
| `vector_pack/search_default` | 1k | 256 | 10 | **21.68 µs** | HNSW. |
| `vector_pack/upsert_default` | 1 | 256 | n/a | 2.56 µs | HNSW single insert. |
| `vector_pack/bulk_upsert_default` | 100 | 256 | n/a | 4.35 ms | HNSW bulk mutation. |
| `vector_pack/ivf_search_default` | 256 | 256 | 10 | **2.92 µs** | Trained IVF; ~7× faster than HNSW at this corpus size. |
| `vector_pack/ivf_bulk_upsert_default` | 100 | 256 | n/a | 60.69 µs | IVF bulk mutation. |
| `vector_pack/ivf_stats_default` | n/a | n/a | n/a | 359.9 ns | Stats read. |

## §7 selene-vector (HNSW + IVF recall + replay)

Registered tokens: `selene-vector:recall:criterion`,
`selene-vector:quant_recall:criterion`, `selene-vector:ivfpq_recall:criterion`,
`selene-vector:composition_replay:criterion`.

### §7a `vector_recall_at_10` (HNSW baseline; ext_select toggle)

| Variant | k=10 | k=25 | k=50 | k=100 |
|---|---:|---:|---:|---:|
| extend_off | 741.7 µs (0.881) | 1.24 ms (0.991) | 2.21 ms (1.000) | 4.60 ms (1.000) |
| extend_on | 709.8 µs (0.881) | 1.23 ms (0.991) | 2.19 ms (1.000) | 4.61 ms (1.000) |

(latency / recall@10; extend_on yields ~4% latency reduction at small k.)

### §7b `quant_recall_at_10` (PQ/SQ/OPQ quantization recall vs f32)

| Variant | k=10 | k=25 | k=50 | k=100 |
|---|---:|---:|---:|---:|
| **f32** (baseline) | 691.1 µs / 0.881 | 1.22 ms / 0.991 | 2.18 ms / 1.000 | 4.59 ms / 1.000 |
| sq8 | 739.7 µs / 0.884 | 1.26 ms / 0.987 | 2.20 ms / 0.994 | 4.64 ms / 0.994 |
| sq8 + rescore | 735.7 µs / 0.884 | 1.27 ms / 0.991 | 2.22 ms / 1.000 | 4.77 ms / 1.000 |
| pq | 731.6 µs / 0.503 | 1.27 ms / 0.506 | 2.30 ms / 0.491 | 4.93 ms / 0.491 |
| pq + rescore | 732.2 µs / 0.503 | 1.26 ms / 0.778 | 2.30 ms / 0.931 | 4.91 ms / **0.984** |
| opq | 721.5 µs / 0.500 | 1.24 ms / 0.472 | 2.30 ms / 0.459 | 4.92 ms / 0.456 |
| opq + rescore | 721.2 µs / 0.500 | 1.25 ms / 0.781 | 2.27 ms / 0.925 | 4.91 ms / **0.994** |

(latency / recall@10. **SQ8** is essentially free. **PQ/OPQ alone** collapse recall to ~50%; pairing with a **rescore** tier recovers recall to 0.98–0.99 at the same wall-clock as f32.)

### §7c `vector_ivfpq_recall_at_10` (IVF-PQ n_probe sweep)

| n_probe | Latency |
|---:|---:|
| 1 | **625.0 ns** |
| 4 | 2.46 µs |
| 8 | 4.63 µs |

Linear in `n_probe`; sub-µs cold-cache lookup at `n_probe=1`.

### §7d `composition_replay` (selene-vector IVF-PQ + snapshot/publish)

| Variant | insert_snapshot_query | insert_publish_query |
|---|---:|---:|
| plain_pq | 23.98 ms | 24.23 ms |
| **opq_polysemous** | **1.16 s** | **1.16 s** |

OPQ rotation on insert is ~48× slower than plain PQ. Significant for insert-heavy workloads — favor `plain_pq` unless polysemous OPQ is needed for the workload's recall target.

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
