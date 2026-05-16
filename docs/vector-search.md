# Vector search

This guide is for engineers adding vector similarity search to an application that
already embeds the `selene-db` graph engine. It covers the two index providers
shipped today (HNSW and IVF-PQ), the configuration knobs, the nine `vector.*`
GQL procedures, the snapshot/recovery story, and the gotchas that show up in
production.

For a high-level overview of the engine, see the workspace
[`README.md`](../README.md). For the GQL surface that `CALL vector.search` plugs
into, see the [GQL reference](gql-reference.md) and the embedding walk-through
in the [embedding guide](embedding-guide.md). Graph algorithms have their own
companion document at [`docs/graph-algorithms.md`](graph-algorithms.md).

## When to use vector search in selene-db

`selene-db` is a property-graph engine first. Vectors are not part of the graph
core — per Architecture Decision D5, anything that is not strictly graph lives
in an opt-in extension crate. Vector search is provided by:

| Crate | Role |
|---|---|
| [`selene-vector`](../crates/selene-vector) | HNSW and IVF-PQ index providers, snapshot bodies, mutation replay, quantization overlays. Implements the `selene_graph::IndexProvider` trait. |
| [`selene-vector-pack`](../crates/selene-vector-pack) | The nine `vector.*` procedure-pack adapters that expose those providers through `CALL` in GQL queries. |

You attach a vector index by registering an `HnswProvider` or `IvfProvider`
with the graph at builder time, and you query and mutate it by calling the
`vector.*` procedures from GQL. The graph core only sees opaque
`Change::IndexExtensionEvent` payloads; it never deserializes vector bodies and
never knows what HNSW or IVF means.

Use the vector extension when you have:

- Dense `f32` embeddings (text, image, audio, structured-feature) attached to
  graph nodes, and
- Top-K nearest-neighbor queries over those embeddings, optionally restricted
  to a graph-derived candidate set (a `RoaringBitmap` of `NodeId` produced by
  some GQL `MATCH`).

If you only need exact lookup, use a typed property index in `selene-graph`
instead — the vector pack is not the right tool for `WHERE node.tag = 'x'`.

## Two index types: HNSW and IVF

Both providers store dense `f32` vectors keyed by `NodeId` and run the same
three distance metrics (`Cosine`, `L2`, `Dot`). They differ in how they trade
recall, latency, memory, and insert cost.

| Property | HNSW | IVF-PQ |
|---|---|---|
| Index structure | Layered proximity graph | Coarse centroids + posting lists of PQ-encoded residuals |
| Insert cost | High per-row (graph rewrite) | Low per-row (centroid assign + PQ encode) |
| Training | None — incremental | Required: K-means coarse + per-subspace PQ codebooks |
| Hot read latency | Microseconds (M5: ~720 µs @ n=10k, k=10, dim=128) | Sub-microsecond at low `n_probe` (M5: ~700 ns @ n=256, dim=256, k=10, n_probe=1) |
| Memory | Roughly `n * dim * 4` bytes plus neighbor lists | Much lower with PQ compression (`compression_ratio` reported by `vector.ivf_stats`) |
| Mutation pattern | Best-effort online insert/delete via `vector.upsert` / `vector.delete` | Deferred buffering until `training_min_vectors` then trained |
| Snapshot footprint | Section tags `GRPH`, `VECS`, optionally `QUNT` | Section tags `CQNT`, `IPQB`, `POST` |

Pick HNSW when recall and latency dominate, the corpus fits in memory, and
inserts are bursty rather than continuous. Pick IVF-PQ when the corpus is
large enough that even SQ8 compression of HNSW would be tight, when inserts
are frequent, and when you can tolerate a training step. The two providers can
coexist on the same graph — IVF lives under provider tag `IVFP`, HNSW under
`VECT` — but each procedure adapter currently targets the single
`'default'` index slot (see the procedure table below).

## Setup

Both crates are workspace members today. Once `selene-db` ships as a crates.io
release, the dependency blocks look like this:

```toml
# Cargo.toml
[dependencies]
selene-core   = { path = "path/to/selene-db/crates/selene-core" }
selene-graph  = { path = "path/to/selene-db/crates/selene-graph" }
selene-gql    = { path = "path/to/selene-db/crates/selene-gql" }
selene-pack   = { path = "path/to/selene-db/crates/selene-pack" }
selene-persist = { path = "path/to/selene-db/crates/selene-persist" }

# Vector extension
selene-vector      = { path = "path/to/selene-db/crates/selene-vector" }
selene-vector-pack = { path = "path/to/selene-db/crates/selene-vector-pack" }
```

The vector extension is fully opt-in. If you don't list it in `[dependencies]`,
the engine knows nothing about HNSW, IVF, quantization, or the `vector.*`
procedures.

Wiring it up is a two-step dance: register the provider with the graph, then
register the procedure pack with the GQL `ProcedurePackRegistry`.

```rust
use std::sync::Arc;

use selene_core::GraphId;
use selene_graph::{IndexProvider, SharedGraph};
use selene_vector::{HnswConfig, HnswProvider};
use selene_vector_pack::VectorPack;

let provider = Arc::new(HnswProvider::new(HnswConfig::new(384)?)?);
let dyn_provider: Arc<dyn IndexProvider> = provider.clone();

let graph = SharedGraph::builder(GraphId::new(1))
    .with_provider(dyn_provider)
    .build()?;

let pack = VectorPack::new();
let registry = pack.registry_with_builtins()?;
```

`registry` is then handed to `selene_gql::analyze` / `plan` / `execute_statement`
in the same way you would for any other procedure pack. See
[`docs/embedding-guide.md`](embedding-guide.md) for the full `Session`
lifecycle.

## HNSW: build, search, mutate

[`HnswConfig`](../crates/selene-vector/src/config.rs) is the central knob set.
`HnswConfig::new(dim)` returns donor-matched defaults; use `with_params` or the
`with_*` builders to override. The validated fields are:

| Field | Type | Default (via `HnswConfig::new`) | Meaning |
|---|---|---|---|
| `dim` | `usize` (1..=`u16::MAX`) | required at construction | Vector dimensionality. Locked for the lifetime of the index. Mismatched inserts fail with `VectorError::DimensionsLocked`. |
| `m` | `usize` (2..=`u16::MAX`) | `16` | Maximum neighbor count per node above layer 0. Larger `m` = higher recall, more memory and slower build. |
| `ef_construction` | `usize` (`>= m`) | `200` | Search width during build. Must be ≥ `m`. Higher = better-built graph, slower inserts. |
| `ef_search` | `usize` (`> 0`) | `50` | Default search-time beam width. Tunable per-query via the optional `ef_search` argument to `vector.search`. Higher = higher recall, higher latency. |
| `metric` | `DistanceMetric` | `Cosine` | One of `Cosine`, `L2`, `Dot`. Scoring is metric-aware throughout — switching after the fact requires rebuilding. |
| `quantization` | `QuantizationConfig` | disabled | See [Quantization](#ivf--pq--opq) below. |
| `neighbor_selection` | `NeighborSelectionConfig` | `extend_candidates: false`, `keep_pruned_connections: true` | HNSW diversity heuristic toggles. `extend_candidates` widens the candidate pool by one hop. `keep_pruned_connections` backfills the neighbor list from rejected candidates. |

The constraint `ef_construction >= m` is enforced at validation time and is the
classic recall-pinning relationship from Malkov and Yashunin 2018.

Constructor signatures:

```rust
// Defaults: m=16, ef_construction=200, ef_search=50, metric=Cosine, no quant.
let cfg = HnswConfig::new(384)?;

// Explicit values.
let cfg = HnswConfig::with_params(384, 32, 400, 100, DistanceMetric::L2)?;

// Add quantization on top of an existing config.
let cfg = HnswConfig::new(384)?
    .with_quantization(QuantizationConfig {
        enabled: true,
        method: QuantMethod::Sq8,
        rescore: true,
        pq: None,
    })?;
```

`HnswProvider::new(cfg)` returns the registerable provider. Direct programmatic
operations are also available on the provider for embedders that do not want to
go through GQL:

```rust
let arc = Arc::new(HnswProvider::new(HnswConfig::new(128)?)?);
// Read-only search:
let hits = arc.search(&query, /* k */ 10, /* ef_search override */ None, None)?;
// Snapshot the current published graph:
let graph_snapshot = arc.snapshot();
```

Mutation is **never** done by mutating the provider in place. All inserts and
deletes flow through the graph's mutation funnel as
`Change::IndexExtensionEvent { provider: "selene-vector", payload: ... }`
records, and the provider replays them via `on_change`. The recommended path is
`CALL vector.upsert` from GQL — see the worked example below.

## HNSW worked example

End-to-end Rust program that:

1. Opens an in-memory graph,
2. Registers a 32-dimensional HNSW provider and the vector pack,
3. Creates 1000 nodes with synthetic random vectors,
4. Runs a top-10 search via `CALL vector.search`.

```rust
use std::sync::Arc;

use selene_core::{GraphId, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::{IndexProvider, SharedGraph};
use selene_gql::{EmptyProcedureRegistry, StatementOutput, analyze, execute_statement, parse, plan};
use selene_vector::{HnswConfig, HnswProvider, VectorOp, VectorUpsertPayloadV1, random_layer_default, HnswParams};
use selene_vector_pack::VectorPack;
use selene_core::Change;
use std::sync::Arc as StdArc;

const DIM: usize = 32;
const N: usize = 1000;

fn make_vector(seed: u64) -> Vec<f32> {
    // Trivial deterministic synthetic embedding; replace with a real model.
    let mut v = vec![0.0f32; DIM];
    for i in 0..DIM {
        v[i] = ((seed.wrapping_mul(2654435761).wrapping_add(i as u64)) as f32).sin();
    }
    v
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build the graph with the HNSW provider attached.
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(DIM)?)?);
    let dyn_provider: Arc<dyn IndexProvider> = provider.clone();
    let graph = SharedGraph::builder(GraphId::new(1))
        .with_provider(dyn_provider)
        .build()?;

    // 2. Register the vector pack to expose `vector.*` procedures over GQL.
    let pack = VectorPack::new();
    let registry = pack.registry_with_builtins()?;
    let item = intern("Item")?;

    // 3. Create 1000 nodes.
    let mut node_ids = Vec::with_capacity(N);
    {
        let mut tx = graph.begin_write();
        let mut mutator = tx.mutator();
        for _ in 0..N {
            node_ids.push(
                mutator.create_node(LabelSet::single(item), PropertyMap::new())?,
            );
        }
        tx.commit()?;
    }

    // 4. Insert vectors via the provider's replay path. In a real app you'd
    //    `CALL vector.upsert(...)` for each row instead; both routes end up
    //    in `HnswProvider::on_change` via the mutation funnel.
    let params = HnswParams::from_config(provider.config());
    for (i, node_id) in node_ids.iter().copied().enumerate() {
        let payload = VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id,
            vector: make_vector(i as u64),
            max_layer: random_layer_default(&mut fastrand::Rng::new(), params.level_factor),
        };
        let bytes = payload.encode()?;
        provider.on_change(&Change::IndexExtensionEvent {
            provider: intern("selene-vector")?,
            payload: StdArc::from(bytes.into_boxed_slice()),
        })?;
    }

    // 5. Run a search via GQL.
    let stmt = parse(
        "CALL vector.search('default', $query, 10, NULL, NULL) YIELD node_id, score"
    )?;
    let analyzed = analyze(stmt, &registry, None)?;
    let planned = plan(&analyzed, &registry)?;
    let mut session = selene_gql::Session::new(&graph);
    session.bind_param(
        intern("query")?,
        Value::List(make_vector(7).into_iter().map(|f| Value::Float(f as f64)).collect()),
    );
    let StatementOutput::Rows(rows) = execute_statement(&planned, &mut session, &registry)? else {
        unreachable!("CALL YIELD always returns rows");
    };
    for row in rows.rows() {
        println!("{:?}", row.values());
    }

    Ok(())
}
```

For the official mutation route — through the procedure pack itself, hitting
the WAL — replace step 4 with a `CALL vector.upsert('default', $node_id, $vec)`
loop inside a write transaction. The `vector_change` helper in
`crates/selene-vector-pack/tests/search.rs` is the canonical reference.

## IVF + PQ + OPQ

The IVF-PQ provider, [`IvfProvider`](../crates/selene-vector/src/ivf/mod.rs),
exposes the inverted-file index with residual product quantization. It
deliberately buffers inserts until enough vectors are present to train both
the coarse centroids and the PQ codebooks, then publishes a trained snapshot.

### What each piece does, in plain English

- **IVF (Inverted File Index).** Partitions vector space into `k_coarse`
  Voronoi cells via K-means. At search time, only the `n_probe` cells closest
  to the query are scanned. Result quality is recall-controlled by `n_probe`;
  latency scales linearly with it (see BENCHMARKS.md §7c — 700 ns at
  `n_probe=1`, 4.97 µs at `n_probe=8`).
- **PQ (Product Quantization).** Splits each `dim`-dimensional vector into
  `m_subspaces` equal sub-vectors and learns 256 centroids per subspace. Each
  vector becomes `m_subspaces` bytes (one centroid id per slice). Distance is
  computed via a lookup table (LUT). PQ alone is a heavy recall hit (see
  BENCHMARKS.md §7b; ~50% recall at k=10 on the bench fixture).
- **OPQ (Optimized PQ).** Learns a `dim × dim` rotation that decorrelates the
  subspaces before PQ training, recovering most of the recall PQ trades away.
  Only enabled by default when `dim <= 1024` (`opq_max_dim()`). Insert cost is
  steep — composition replay shows OPQ + polysemous at ~48× plain PQ
  (BENCHMARKS.md §7d).
- **Polysemous codes.** A bit-permutation on top of trained PQ centroids that
  lets a Hamming pre-filter on the query / candidate code prune candidates
  before LUT scoring. Default-on when `m_subspaces >= 2`; opt out via
  `PqParams { use_polysemous: false, ... }`.

The IVF config knobs:

| Field | Default (via `IvfConfig::new`) | Meaning |
|---|---|---|
| `dim` | required | Vector dimensionality, capped at `u16::MAX`. |
| `k_coarse` | `256` | Number of coarse centroids / posting lists. Validated to `1..=65_536`. |
| `n_probe` | `ceil(sqrt(k_coarse))` (16 for k=256) | Default posting lists scanned per query. Tunable per-call. |
| `metric` | `L2` | `Cosine`, `L2`, or `Dot`. |
| `pq` | `PqParams::default_for_dim(dim)` | Subspace count, centroid count (locked to 256), OPQ toggle, polysemous toggle, Hamming threshold ratio. |
| `training_min_vectors` | `max(k_coarse * 39, pq.train_min_vectors)` | Lower bound on buffered vectors before training fires. |

Quantization on HNSW is configured via `HnswConfig::with_quantization` or
`with_pq_quantization`. The QUNT body is an **optional, post-commit snapshot
overlay** — you can enable quantization on an existing HNSW provider without
rebuilding from scratch; the next snapshot cycle just adds the QUNT section.
`QuantizationConfig` carries:

| Field | Meaning |
|---|---|
| `enabled` | When true, the next snapshot writes the QUNT body. |
| `method` | `Sq8` (per-coordinate scalar) or `Pq` (product). |
| `rescore` | When true, the quantized search returns an over-fetched candidate set that is then re-ranked with exact f32 scoring. Recovers most of the PQ recall loss at minimal latency cost — see BENCHMARKS.md §7b. |
| `pq` | Optional `PqParams` override for `method = Pq`. |

The validator rejects `rescore = true` while `enabled = false`, and validates
PQ dimension divisibility (`dim` must be divisible by `m_subspaces`).

## Asymmetric search with LUTs

When a quantized store is loaded (HNSW with `QuantizationConfig { enabled: true,
.. }` or IVF-PQ in trained mode), search is **asymmetric**: the query stays as
full-precision f32, the stored vectors are quantized codes, and distances are
computed by building a query-side lookup table (LUT) once per query and then
summing LUT entries indexed by the stored codes.

This is what makes PQ/OPQ fast: the inner loop is one `f32` add per subspace
per candidate, not a full distance computation. No extra knob is needed to
enable LUT mode — when the QUNT or IPQB body is present, the provider's
search path uses LUTs automatically. Use `rescore: true` to recover recall on
the over-fetched candidate set.

For the precise LUT path see
[`crates/selene-vector/src/quantize/mod.rs`](../crates/selene-vector/src/quantize/mod.rs)
(method `build_query_lut_into`).

## The 9 vector.* procedures

All vector procedures register under the static `VECTOR_PROCEDURE_NAMES` list
in [`crates/selene-vector-pack/src/registry.rs`](../crates/selene-vector-pack/src/registry.rs).
Every procedure takes `index_name` as its first argument; in v1.0 only the
literal `'default'` is accepted (`reject_non_default_index`). Multi-index
support is reserved for a future version.

| Procedure | Tier | Signature | Output columns | Purpose |
|---|---|---|---|---|
| `vector.search` | read | `(index_name: STRING, query: LIST<FLOAT>, k: INT, ef_search: INT?, filter_nodes: LIST<NODEREF>?)` | `(node_id: NODEREF, score: FLOAT)` | HNSW top-K nearest neighbor search. Optional bitmap pre-filter from a graph-derived candidate set. |
| `vector.upsert` | mutation | `(index_name: STRING, node_id: NODEREF, vector: LIST<FLOAT>)` | none | Single-row HNSW insert. Vector dimension must match the configured `dim`. NaN / infinity rejected. |
| `vector.delete` | mutation | `(index_name: STRING, node_id: NODEREF)` | none | Single-row HNSW tombstone delete. Missing IDs are no-ops. |
| `vector.bulk_upsert` | mutation | `(index_name: STRING, node_ids: LIST<NODEREF>, vectors: LIST<LIST<FLOAT>>)` | none | Batched HNSW insert. All vectors must share `dim`. Duplicate `node_id` in the batch or against existing rows is rejected. |
| `vector.bulk_delete` | mutation | `(index_name: STRING, node_ids: LIST<NODEREF>)` | none | Batched HNSW tombstone delete. Empty list rejected. Duplicates in the batch rejected. Entry-point recomputation runs once per batch. |
| `vector.ivf_search` | read | `(index_name: STRING, query: LIST<FLOAT>, k: INT, n_probe: INT?, filter_nodes: LIST<NODEREF>?)` | `(node_id: NODEREF, score: FLOAT)` | IVF-PQ top-K search. `n_probe` overrides the configured default. |
| `vector.ivf_bulk_upsert` | mutation | `(index_name: STRING, node_ids: LIST<NODEREF>, vectors: LIST<LIST<FLOAT>>)` | none | Batched IVF insert. Pre-training the rows buffer; post-training they are encoded into posting lists. |
| `vector.ivf_bulk_delete` | mutation | `(index_name: STRING, node_ids: LIST<NODEREF>)` | none | Batched IVF tombstone delete. |
| `vector.ivf_stats` | read | `(index_name: STRING)` | 14-column row covering `state` ∈ {`trained`, `deferred`}, `k_coarse`, `n_probe_default`, `posting_list_lengths`, `unassigned_count`, `bytes_coarse_centroids`, `bytes_residual_codebook`, `bytes_rotation`, `bytes_posting_lists`, `bytes_reconstructed_norms`, `compression_ratio`, `polysemous`, `observed_vectors`, `required` | Introspect an IVF index. In the `deferred` (pre-training) state only `observed_vectors` and `required` are populated; in `trained` state the byte-accounting fields are populated and the deferred fields are NULL. |

The `vector.upsert` procedure pack carries a `VectorPackConfig` —
specifically the optional `deterministic_seed: Option<u64>`. When set, every
row's HNSW layer assignment becomes deterministic given the input `NodeId` and
seed, so snapshot bytes are reproducible across processes. Construct with
`VectorPack::with_config(VectorPackConfig { deterministic_seed: Some(0xc0ffee), .. })`.

## GQL surface

The procedures plug into the standard `CALL ... YIELD ...` form. Examples:

```gql
-- Top-10 over the default HNSW index, with a parameter-bound query vector.
CALL vector.search('default', $query_vec, 10, NULL, NULL)
YIELD node_id, score
RETURN node_id, score
ORDER BY score DESC;

-- Override ef_search at query time for higher recall.
CALL vector.search('default', $query_vec, 50, 200, NULL)
YIELD node_id, score
RETURN node_id, score;

-- Restrict to a graph-derived candidate set: first MATCH the eligible nodes,
-- then pass them as `filter_nodes`. Selene's pre-filter is a RoaringBitmap so
-- thousands of NodeRefs are cheap.
MATCH (item:Item) WHERE item.in_stock = true
WITH collect(item) AS candidates
CALL vector.search('default', $query_vec, 10, NULL, candidates)
YIELD node_id, score
RETURN node_id, score;

-- IVF search, increasing recall by widening n_probe.
CALL vector.ivf_search('default', $query_vec, 10, 8, NULL)
YIELD node_id, score
RETURN node_id, score;

-- Mutations from GQL.
CALL vector.upsert('default', $node_id, $vec);
CALL vector.bulk_upsert('default', $node_ids, $vectors);
CALL vector.delete('default', $node_id);

-- Introspect an IVF index.
CALL vector.ivf_stats('default')
YIELD state, k_coarse, n_probe_default, posting_list_lengths, compression_ratio
RETURN state, k_coarse, n_probe_default, posting_list_lengths, compression_ratio;
```

Parameter binding flows through the standard GQL session API. See the
[embedding guide](embedding-guide.md) for how to bind a `LIST<FLOAT>` from a
Rust `Vec<f32>`.

## Performance

Headline numbers from [`BENCHMARKS.md`](../BENCHMARKS.md) measured on Apple
M5, macOS 26.5, on 2026-05-16. All figures are committed to that file and
should be re-read there for the freshest data.

| Workload | Scale | Median latency | Notes |
|---|---|---|---|
| `vector_pack/search_default` | n=1000, dim=256, k=10 | **18.51 µs** | HNSW search through the GQL `CALL` adapter (parse + plan + execute). |
| `vector_pack/upsert_default` | dim=256 | **2.42 µs** | HNSW single-row insert. |
| `vector_pack/bulk_upsert_default` | 100 rows, dim=256 | **4.22 ms** | HNSW bulk insert. |
| `vector_pack/ivf_search_default` | n=256, dim=256, k=10 | **2.88 µs** | Trained IVF, ~6× faster than HNSW at this corpus size. |
| `vector_pack/ivf_bulk_upsert_default` | 100 rows, dim=256 | **57.37 µs** | IVF bulk insert (pre-training buffer). |
| `vector_pack/ivf_stats_default` | n/a | **358.9 ns** | Stats read. |
| `vector_hnsw_build` | n=1000, dim=16, M=8, ef_construction=64 | **53.25 ms** | Direct `insert_node` cold build. |
| `vector_recall_at_10` (HNSW f32 baseline) | n=10000, dim=128, k=10 | 718.3 µs / recall 0.881 | `vector_recall_at_10` fixture, M5 §7a. |
| `quant_recall_at_10` (SQ8 + rescore) | same as above | 749.3 µs / recall 0.884 | SQ8 is essentially free latency-wise; `rescore` keeps recall on par with f32. |
| `quant_recall_at_10` (PQ + rescore @ k=100) | same | 4.92 ms / recall 0.984 | PQ alone collapses recall to ~50%; pairing with `rescore` recovers it. |
| `vector_ivfpq_recall_at_10` | dim=128, n_probe=1 | **699.7 ns** | Sub-µs IVF-PQ cold lookup. |

The headline lesson: **always pair PQ or OPQ with `rescore: true`** unless you
have explicitly measured recall on your workload. SQ8 is the safest opt-in
quantization choice when you only need modest compression.

## Recovery semantics

When an HNSW or IVF provider is registered with a `SharedGraph`, its state
participates in the engine's two-step snapshot/replay recovery pipeline:

- The provider publishes immutable snapshot bodies through `ArcSwap`. During
  snapshot write, the provider emits its sections in a stable declared order
  (HNSW: `GRPH`, `VECS`, optionally `QUNT`; IVF: `CQNT`, `IPQB`, `POST`).
- The persistence crate writes those sections into the `SLSN` snapshot
  alongside graph and pack sections, then continues capturing
  `Change::IndexExtensionEvent` records in the `SLDB` WAL.
- On startup, the WAL is replayed, then the snapshot is loaded; the provider
  decodes its sections in the same declared order and resumes from the
  recovered state. Post-snapshot WAL events replay through `on_change`.

The HNSW snapshot wire bodies live under
[`crates/selene-vector/src/snapshot/`](../crates/selene-vector/src/snapshot/):

| Tag | Provider | Role |
|---|---|---|
| `GRPH` (`VGRP`/`VGP2` payload magic) | `VECT` | HNSW topology — nodes, neighbor lists, entry point. |
| `VECS` (`VVEC` payload magic) | `VECT` | Raw `f32` vector payload, dense by `InternalIndex`. |
| `QUNT` (`VQNT` payload magic) | `VECT` | Optional quantized store overlay (SQ8 or PQ codes, codebook, optional OPQ rotation). |
| `CQNT` (`VCQB` payload magic) | `IVFP` | IVF coarse quantizer centroids. |
| `IPQB` (`VIPB` payload magic) | `IVFP` | IVF residual PQ codebook (with optional OPQ rotation and polysemous permutation). |
| `POST` (`VPOS` payload magic) | `IVFP` | IVF posting lists keyed by coarse centroid. |

Because QUNT is optional, you can enable, disable, or re-train quantization
without touching the graph topology — the next snapshot just adjusts the
QUNT section. The provider tag constants live in
[`crates/selene-vector/src/snapshot/tags.rs`](../crates/selene-vector/src/snapshot/tags.rs).

## Limits and gotchas

- **Dimension cap.** `dim` is stored as `u16` in the wire format. The
  validated upper bound is `u16::MAX = 65535`. Most embedding models stay well
  inside this — e.g. OpenAI `text-embedding-3-large` is 3072.
- **Dimension is locked.** Once a provider is constructed with a given `dim`,
  every insert must match. Length mismatch returns
  `VectorError::DimensionsLocked { expected, observed }`. Migrating to a
  different dim requires a fresh provider (and a fresh index slot).
- **NaN and infinity are rejected.** All `vector.upsert`-tier procedures
  validate every component of every input vector. The provider would crash
  the search heap if NaN slipped through; reject early.
- **Vector count cap.** The HNSW provider addresses rows via `InternalIndex =
  u32`. The hard ceiling is `u32::MAX` live rows per provider. The
  `ensure_bulk_capacity` preflight rejects bulk inserts that would project
  past this.
- **Tombstones, not in-place updates.** `vector.delete` tombstones the row;
  `vector.upsert` rejects re-inserting an existing `node_id` with
  `VectorError::DuplicateNodeId`. The `VectorOp::Update` opcode is reserved
  for a future brief and currently returns `OperationNotSupportedYet`.
- **Only the `'default'` index is wired in v1.0.** All nine procedures call
  `reject_non_default_index` on argument 0. Multi-index support is reserved
  for a future minor.
- **IVF is deferred until trained.** Until `training_min_vectors` are
  buffered, `vector.ivf_search` returns no rows and `vector.ivf_stats` reports
  `state = 'deferred'` with `observed_vectors` and `required` populated.
  Trigger training by pushing enough rows through `vector.ivf_bulk_upsert`.
- **OPQ is heavy on insert.** `composition_replay` (BENCHMARKS.md §7d) shows
  `opq_polysemous` at ~1.17 s per cycle vs `plain_pq` at ~25 ms — favor
  `plain_pq` unless polysemous OPQ recall is required.
- **The graph core never sees vector internals.** Every mutation crossing
  the provider boundary is a `Change::IndexExtensionEvent { provider: "selene-vector", payload: Arc<[u8]> }`
  (or `provider: "selene-vector-ivf"` for IVF). If you observe `on_change`
  receiving a payload tagged for a different provider, the provider correctly
  ignores it.

## See also

- [`docs/embedding-guide.md`](embedding-guide.md) — how to wire `SharedGraph`,
  `Session`, `ProcedurePackRegistry`, and parameter binding end-to-end.
- [`docs/gql-reference.md`](gql-reference.md) — the GQL surface, including
  parameter binding and `CALL ... YIELD ...` shape.
- [`docs/graph-algorithms.md`](graph-algorithms.md) — algorithms over a
  `GraphProjection`, the sister extension crate to `selene-vector`.
- [`crates/selene-vector/src/lib.rs`](../crates/selene-vector/src/lib.rs) —
  the public Rust API surface.
- [`BENCHMARKS.md`](../BENCHMARKS.md) — committed performance baselines.
