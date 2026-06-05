# selene-db

`selene-db` is an embeddable Rust property graph engine built around ISO/IEC
39075:2024 GQL, native graph algorithms, first-class vectors, and BM25 text
search.

The project is greenfield and library-first. There is no server, no transport
layer, no auth layer, and no loadable extension or procedure-pack system.
Embedders link the workspace crates, own their application boundary, and run
the engine in-process.

## What It Is

- **Strict ISO GQL engine.** Parser, semantic analyzer, planner, optimizer, and
  row executor are centered on ISO/IEC 39075:2024 GQL. Non-standard grammar is
  rejected rather than accepted as Cypher, SQL, or SPARQL drift.
- **Single native graph engine.** The graph store, persistence layer, procedure
  registry, algorithms, vector indexes, and text indexes are cohesive engine
  features, not optional plug-ins.
- **Agentic-memory substrate.** Vectors and BM25 are native because real users
  are using the graph for retrieval, memory, and agentic AI workloads, while ISO
  GQL remains the query-language contract.
- **Embeddable Rust workspace.** The public surface is crates, not a hosted
  service. Applications choose their own network, tenancy, auth, and deployment
  model.

## Current Surface

| Area | Current implementation |
| --- | --- |
| Graph storage | In-memory property graph with copy-on-write snapshots, `ArcSwap`, `parking_lot::RwLock`, `imbl`, Roaring label indexes, typed indexes, composite indexes, and one mutation funnel. |
| Transactions | Strict-serializable writes under one graph write lock with lock-free read snapshots. |
| Persistence | Graph-blind WAL (`SLDB`), snapshots (`SLSN`), MANIFEST-led recovery, retention pruning, and an append-only audit log substrate. |
| GQL | ISO-oriented parser, semantic analyzer, planner, optimizer, executor, plan cache, built-in procedure registry, and GQL Flagger enforcement for unsupported constructs. |
| Procedures | One frozen native `BuiltinProcedureRegistry`: 52 procedures total, including 19 `algo.*` graph algorithms and 33 `selene.*` platform/vector/text procedures. |
| Graph algorithms | Structural, pathfinding, centrality, and community algorithms exposed through the native Rust API and ISO `CALL algo.*`. |
| Vectors | `Value::Vector` as a first-class value, exact vector scoring/search, HNSW and IVF indexes, ANN search, batch search, candidate expansion, and candidate-state scoring APIs. |
| Text search | Native BM25 text indexes over string node properties, global search, candidate-scoped scoring, `CALL selene.text_search_nodes`, and `CALL selene.text_score_nodes`. |
| Safety posture | Workspace-wide `#![forbid(unsafe_code)]`, `missing_docs = "deny"`, rustls-only dependency policy, and a 700 LOC source-file cap. |

## Workspace Layout

| Crate | Purpose |
| --- | --- |
| [`selene-core`](crates/selene-core) | Foundation types: `Value`, `VectorValue`, `IStr`, `PropertyMap`, `LabelSet`, schema types, codecs, origins, and changesets. |
| [`selene-graph`](crates/selene-graph) | Storage engine, mutation funnel, graph type binding, label/typed/composite indexes, vector indexes, BM25 text indexes, candidate state, and recovery providers. |
| [`selene-persist`](crates/selene-persist) | WAL, snapshots, MANIFEST recovery, pruning, and audit log files. This crate stays graph-blind. |
| [`selene-algorithms`](crates/selene-algorithms) | Mandatory graph algorithm crate with projection catalogs, free functions, and the `GraphAlgorithms` convenience trait. |
| [`selene-gql`](crates/selene-gql) | GQL grammar, AST, analysis, planning, optimization, execution, and the native built-in procedure registry. |
| [`selene-testing`](crates/selene-testing) | Test fixtures, synthetic graph generators, and pure-mirror snapshot-harness DSLs for crate integration tests. |

Dependency direction is intentionally narrow: `core -> graph -> algorithms ->
gql`, with `persist` below graph storage and `testing` used as a dev-dependency.
There is no umbrella crate.

## Quickstart

Add the crates you need as path dependencies while the project is private:

```toml
[dependencies]
selene-core = { path = "path/to/selene-db/crates/selene-core" }
selene-graph = { path = "path/to/selene-db/crates/selene-graph" }
selene-gql = { path = "path/to/selene-db/crates/selene-gql" }
```

Create a graph, insert a node through the native write API, then query it with
GQL:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, intern};
use selene_gql::{BuiltinProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = SharedGraph::new(GraphId::new(1));

    let person = intern("Person")?;
    let name = intern("name")?;

    let mut tx = graph.begin_write();
    let mut props = PropertyMap::new();
    props.set(name, Value::String(intern("Ada")?))?;
    tx.mutator().create_node(LabelSet::single(person), props)?;
    tx.commit()?;

    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let output = session.execute_source("MATCH (p:Person) RETURN p.name", &registry)?;

    let StatementOutput::Rows(rows) = output else {
        panic!("query should return rows");
    };
    assert_eq!(rows.row_count(), 1);

    Ok(())
}
```

See [docs/getting-started.md](docs/getting-started.md) and
[docs/embedding-guide.md](docs/embedding-guide.md) for a longer embedding walk
through.

## GQL And Native Procedures

GQL is the only query and mutation language. Native engine capabilities that
sit outside mandatory ISO syntax are exposed through ISO `CALL`, not through
grammar extensions.

Examples of the current built-in surface:

```gql
CALL selene.health()
CALL selene.feature_status()
CALL selene.create_index(...)
CALL selene.create_vector_index(...)
CALL selene.vector_search_nodes_ann(...)
CALL selene.create_text_index(...)
CALL selene.text_search_nodes(...)
CALL selene.text_score_nodes(...)
CALL algo.pagerank(...)
```

The registry currently contains 19 `algo.*` graph algorithm procedures and 33
`selene.*` platform/vector/text procedures. The registry is native and frozen at
construction time; it is not a third-party extension mechanism.

## Vectors And Text Search

Vectors are real engine values, not sidecar metadata. A node property can hold a
`Value::Vector`, and the graph layer can maintain exact and approximate indexes
over vector-bearing properties.

The vector surface includes:

- Exact top-k scoring and search.
- HNSW and IVF indexes for approximate nearest-neighbor search.
- Batch search and scoring calls.
- Candidate-scoped search and graph-expanded candidate scoring.
- Index memory accounting and rebuild procedures.

Text search is native BM25 over string node properties. It supports global
search and candidate-scoped search, which is useful for hybrid retrieval paths
such as graph-filtered BM25, BM25 reranking over vector roots, and vector rerank
over text or graph candidates.

## ISO GQL Posture

`selene-db` targets minimum ISO/IEC 39075:2024 conformance plus a curated set of
optional features. The feature register in `selene-core` is the source of truth
for supported optional features, and the GQL Flagger rejects unsupported or
non-standard constructs at parse time.

Important boundaries:

- Mandatory ISO scalar types are `STRING`, `BOOLEAN`, `INT`, and `FLOAT`.
- Open and closed graph modes are supported.
- Default isolation is serializable; the implementation provides
  strict-serializable behavior.
- No wire format is specified by ISO GQL, and this repo intentionally does not
  ship one.
- Auth, tenancy, and network policy are embedder responsibilities.

## Development

The common local validation set is:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --all-features --profile default
cargo test --workspace --locked --all-features --doc
cargo deny check bans licenses sources
cargo audit -d /private/tmp/selene-advisory-db
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
bash .github/scripts/check-thirdparty-current.sh
```

Benchmarks run through the serialized helper, not `cargo bench --workspace`:

```bash
scripts/run-benches.sh --profile quick --layer criterion
scripts/run-benches.sh --profile full --layer criterion
scripts/run-benches.sh --profile quick --layer iai
```

See [BENCHMARKS.md](BENCHMARKS.md), [docs/performance.md](docs/performance.md),
and [docs/contributing.md](docs/contributing.md) for current benchmark and
workflow details.

## Documentation

- [Getting started](docs/getting-started.md)
- [Embedding guide](docs/embedding-guide.md)
- [GQL reference](docs/gql-reference.md)
- [Architecture](docs/architecture.md)
- [Graph algorithms](docs/graph-algorithms.md)
- [Persistence and recovery](docs/persistence-and-recovery.md)
- [Observability](docs/observability.md)
- [Performance](docs/performance.md)
- [Contributing](docs/contributing.md)

## Platform Support

| Platform | Status |
| --- | --- |
| Linux x86_64 / aarch64 | Primary deployment target. |
| macOS Apple Silicon / Intel | Primary development target. |
| Windows | Out of scope for now. |

## License

Dual-licensed under MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE), [NOTICE](NOTICE), and
[THIRDPARTY.md](THIRDPARTY.md).
