# selene-db benchmarks

_Last measured: TBD on TBD hardware_

> Methodology: `scripts/run-benches.sh --profile full --layer criterion`
> (sequential execution; concurrent `cargo bench` is blocked by the script's
> `pgrep` guard). Medians are criterion 0.8 wall-clock. iai-callgrind
> instruction-count baselines are deferred to the v1.0.0 release-prep
> panic-audit pass.

## Hardware footprint capture

Run before each measurement pass and paste into the file header:

| Field | Value | Source |
|---|---|---|
| CPU | TBD | `sysctl -n machdep.cpu.brand_string` (macOS) / `lscpu` (Linux) |
| Cores | TBD | `sysctl -n hw.physicalcpu hw.logicalcpu` / `nproc --all` |
| Memory | TBD | `sysctl -n hw.memsize` / `free -h` |
| OS | TBD | `sw_vers` / `uname -srm` |
| rustc | TBD | `rustc --version` |
| Commit | TBD | `git rev-parse --short HEAD` |

## §1 selene-graph hot paths

Registered tokens:

| Token | Bench | Scale | Median | Notes |
|---|---|---:|---:|---|
| `selene-graph:single_graph:criterion` | single_graph::node_fetch | 10k | TBD | |
| `selene-graph:single_graph:criterion` | single_graph::node_fetch | 50k | TBD | |
| `selene-graph:single_graph:criterion` | single_graph::node_fetch | 100k | TBD | |
| `selene-graph:single_graph:criterion` | single_graph::label_scan | 10k / 50k / 100k | TBD | |
| `selene-graph:single_graph:criterion` | single_graph::typed_index_lookup | 10k / 50k / 100k | TBD | |
| `selene-graph:bulk_mutation:criterion` | bulk_mutation::create_nodes | 10k / 50k / 100k | TBD | |
| `selene-graph:concurrent_read:criterion` | concurrent_read::snapshot_reads | 10k / 50k / 100k | TBD | ArcSwap flat-curve signal. |
| `selene-graph:bfs:criterion` | bfs::depth_sweep | 10k / 50k / 100k | TBD | |

## §2 selene-persist

| Token | Bench | Scale | Median | Notes |
|---|---|---:|---:|---|
| `selene-persist:wal:criterion` | wal::append_single | 10k / 50k / 100k | TBD | Scale is WAL entries, not graph nodes. |
| `selene-persist:wal:criterion` | wal::append_batch_1000 | 10k / 50k / 100k | TBD | Scale is WAL entries. |
| `selene-persist:wal:criterion` | wal::replay | 10k / 50k / 100k | TBD | Scale is WAL entries. |
| `selene-persist:snapshot:criterion` | snapshot::write | 10k / 50k / 100k | TBD | Scale is section-size envelope. |
| `selene-persist:snapshot:criterion` | snapshot::read | 10k / 50k / 100k | TBD | |
| `selene-persist:snapshot:criterion` | snapshot::full_recovery | 10k / 50k / 100k | TBD | |

## §3 selene-gql (scale-independent)

| Token | Bench | Median | Notes |
|---|---|---:|---|
| `selene-gql:parse:criterion` | parse::simple_match | TBD | Single-query latency. |
| `selene-gql:analyze:criterion` | analyze::correlated_exists | TBD | Donor regression floor: <1 ms. |
| `selene-gql:plan_optimize:criterion` | plan_optimize::cached_plan | TBD | Planner/optimizer latency. |

## §4 selene-algorithms (Sequential vs Auto)

| Token | Bench | Scale | Sequential | Auto | Notes |
|---|---|---:|---:|---:|---|
| `selene-algorithms:algo_bench:criterion` | pagerank | 10k | TBD | TBD | DESC by score + NodeId ASC; V169 contract. |
| `selene-algorithms:algo_bench:criterion` | pagerank | 50k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | pagerank | 100k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | betweenness | 10k | TBD | TBD | Endpoint-aware sampling; V168 contract. |
| `selene-algorithms:algo_bench:criterion` | betweenness | 50k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | betweenness | 100k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | triangle_count | 10k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | triangle_count | 50k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | triangle_count | 100k | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | apsp | 200 | TBD | TBD | All-pairs SSSP; scale = source count. |
| `selene-algorithms:algo_bench:criterion` | apsp | 500 | TBD | TBD | |
| `selene-algorithms:algo_bench:criterion` | apsp | 1k | TBD | TBD | Stage 3 calibration: 34.0 ms Sequential / 8.45 ms Auto at 1k; cap kept at 1k. |
| `selene-algorithms:algo_bench:criterion` | louvain | 10k | TBD | n/a | Sequential-only in v1.0. |
| `selene-algorithms:algo_bench:criterion` | louvain | 50k | TBD | n/a | |
| `selene-algorithms:algo_bench:criterion` | louvain | 100k | TBD | n/a | Stage 3 calibration: 55.9 ms at 100k; no scale downgrade. |

## §5 selene-algorithms-pack (adapter overhead)

| Token | Bench | Fixture | Median | Notes |
|---|---|---|---:|---|
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/projection_build_default | 1k deterministic directed graph | TBD | |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_pagerank_default | 256-node prebuilt projection | TBD | |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_dijkstra_single_pair | 256-node prebuilt projection | TBD | |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_apsp_default | 96-node prebuilt projection | TBD | Small-N APSP. |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_betweenness_default | 256-node prebuilt projection | TBD | |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_louvain_default | 256-node prebuilt projection | TBD | |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_triangle_count_default | 256-node prebuilt projection | TBD | |
| `selene-algorithms-pack:algo_pack:criterion` | algo_pack/algo_label_propagation_default | 256-node prebuilt projection | TBD | |

## §6 selene-vector-pack (GQL CALL adapter overhead)

| Token | Bench | Vector count | Dim | k | Median | Notes |
|---|---|---:|---:|---:|---:|---|
| `selene-vector-pack:vector_pack:criterion` | search_default | 1k | 256 | 10 | TBD | HNSW. |
| `selene-vector-pack:vector_pack:criterion` | upsert_default | 1 | 256 | n/a | TBD | HNSW mutation. |
| `selene-vector-pack:vector_pack:criterion` | bulk_upsert_default | 100 | 256 | n/a | TBD | HNSW bulk mutation. |
| `selene-vector-pack:vector_pack:criterion` | ivf_search_default | 256 | 256 | 10 | TBD | Trained IVF. |
| `selene-vector-pack:vector_pack:criterion` | ivf_bulk_upsert_default | 100 | 256 | n/a | TBD | IVF bulk mutation. |

## §7 selene-vector (HNSW + IVF recall + replay)

| Token | Bench | Vectors | Dim | Median | Notes |
|---|---|---:|---:|---:|---|
| `selene-vector:recall:criterion` | recall::hnsw_k10 | TBD | TBD | TBD | |
| `selene-vector:quant_recall:criterion` | quant_recall::pq | TBD | TBD | TBD | |
| `selene-vector:ivfpq_recall:criterion` | ivfpq_recall::ivf_pq | TBD | TBD | TBD | |
| `selene-vector:composition_replay:criterion` | composition_replay::hnsw_pq | TBD | TBD | TBD | |

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
