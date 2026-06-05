# selene-db

`selene-db` is an embeddable Rust property graph engine built around
ISO/IEC 39075:2024 GQL, strict in-process transactions, native graph
algorithms, first-class dense vectors, and BM25 text search.

The project is greenfield and library-first. There is no database server,
wire protocol, auth layer, cloud control plane, extension loader, or procedure
pack system in this repository. Applications link the crates they need, own the
network and security boundary, and run the engine in process.

## What This Project Is

`selene-db` is a single native graph engine:

- GQL is the query and mutation language.
- The storage engine is an in-memory property graph with copy-on-write
  snapshots and one mutation funnel.
- Persistence is built from graph-blind WAL and snapshot providers.
- Graph algorithms are mandatory engine code, exposed as Rust APIs and through
  ISO `CALL algo.*`.
- Vectors and BM25 are native engine features because the primary workloads are
  retrieval, agentic memory, and graph-backed AI systems.

The north star is ISO GQL conformance plus pragmatic native retrieval features.
Non-standard capabilities are exposed through implementation-defined procedure
hooks, not by extending the GQL grammar.

## Engine Surface

| Area | Current shape |
| --- | --- |
| Storage | In-memory property graph with `ArcSwap` read snapshots, `parking_lot` write serialization, `imbl` copy-on-write maps, Roaring label indexes, typed indexes, composite indexes, stable external node/edge IDs, and dense internal row indexes. |
| Transactions | Strict-serializable behavior: one writer at a time per graph, lock-free immutable reader snapshots, rollback by non-publication, and provider fanout under the write lock. |
| Persistence | Graph-blind WAL (`SLDB`), snapshots (`SLSN`), MANIFEST-led recovery, retention pruning, and an append-only audit log substrate. |
| GQL | ISO-oriented parser, semantic analyzer, planner, optimizer, executor, plan cache, parameter binding, built-in procedure registry, and GQL Flagger checks for unsupported syntax. |
| Procedures | One native `BuiltinProcedureRegistry` for platform procedures, vector search/scoring, BM25 search/scoring, index metadata, and `algo.*` graph algorithms. |
| Algorithms | Structural, pathfinding, centrality, and community algorithms via `selene-algorithms` and `CALL algo.*`. |
| Vectors | `Value::Vector` as a real value variant, exact scoring/search, HNSW indexes, IVF indexes, ANN batch search, graph-expanded candidate scoring, maintained candidate state, and index memory/rebuild accounting. |
| Text | Maintained BM25 text indexes over string node properties, global text search, candidate-scoped BM25 scoring, and text index stats. |
| Safety | Workspace-wide `#![forbid(unsafe_code)]`, `missing_docs = "deny"`, rustls-only dependency posture, source file-size gates, secret scans, license checks, fuzz targets, and benchmark hygiene checks. |

## Workspace Crates

There is no umbrella crate. Use the layers directly.

| Crate | Owns |
| --- | --- |
| [`selene-core`](crates/selene-core) | Foundation types: `Value`, `VectorValue`, IDs, `IStr`, property maps, label sets, schema metadata, codecs, origins, and changesets. |
| [`selene-graph`](crates/selene-graph) | The graph runtime: storage, transactions, mutation funnel, indexes, vector search, BM25 text search, candidate state, graph type validation, compaction, and recovery providers. |
| [`selene-persist`](crates/selene-persist) | WAL, snapshots, MANIFEST recovery, retention pruning, and audit log files. This crate stays below graph semantics. |
| [`selene-algorithms`](crates/selene-algorithms) | Native graph algorithms, projection catalogs, free functions, and the `GraphAlgorithms` convenience trait. |
| [`selene-gql`](crates/selene-gql) | GQL grammar, AST, analysis, planning, optimization, execution, procedure traits, and the built-in registry. |
| [`selene-testing`](crates/selene-testing) | Test fixtures, synthetic graph generators, benchmark corpora, and pure-mirror snapshot harness utilities. |

Dependency direction stays narrow: `core -> graph -> algorithms -> gql`, with
`persist` graph-blind and `testing` used only from tests and benchmarks.

## Quickstart

While the repository is private and unpublished, depend on crates by path:

```toml
[dependencies]
selene-core = { path = "../selene-db/crates/selene-core" }
selene-graph = { path = "../selene-db/crates/selene-graph" }
selene-gql = { path = "../selene-db/crates/selene-gql" }
```

Create a graph, write through the transaction funnel, and query through GQL:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, intern};
use selene_gql::{BuiltinProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = SharedGraph::new(GraphId::new(1));

    let person = intern("Person")?;
    let name = intern("name")?;

    let mut tx = graph.begin_write();
    let props = PropertyMap::from_pairs([(
        name,
        Value::String(intern("Ada")?),
    )])?;
    tx.mutator().create_node(LabelSet::single(person), props)?;
    tx.commit()?;

    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let output = session.execute_source(
        "MATCH (p:Person) RETURN p.name AS name",
        &registry,
    )?;

    let StatementOutput::Rows(rows) = output else {
        panic!("MATCH ... RETURN should return rows");
    };
    assert_eq!(rows.row_count(), 1);

    Ok(())
}
```

For a longer walk through graph creation, transactions, parameters, and
persistence wiring, start with [docs/getting-started.md](docs/getting-started.md)
and [docs/embedding-guide.md](docs/embedding-guide.md).

## GQL And Procedures

GQL is the only engine query language. The parser rejects SQL, Cypher, SPARQL,
and other grammar drift instead of accepting them as aliases.

Native engine capabilities outside mandatory ISO syntax use ISO `CALL`:

```gql
CALL selene.health()
CALL selene.feature_status()
CALL selene.create_index(...)
CALL selene.create_vector_index(...)
CALL selene.vector_search_nodes_ann(...)
CALL selene.vector_score_expanded_candidates(...)
CALL selene.create_text_index(...)
CALL selene.text_search_nodes(...)
CALL selene.text_score_nodes(...)
CALL selene.text_score_nodes_batch(...)
CALL algo.pagerank(...)
```

The production registry is `BuiltinProcedureRegistry`. It is native engine
code, fixed at construction time, and not a third-party extension mechanism.
Tests can still inject alternate `ProcedureRegistry` implementations where the
planner or executor needs an artificial procedure surface.

## Vectors, Text, And Graph Retrieval

Vectors are first-class values. A node property can hold `Value::Vector`, and
the graph layer can maintain vector indexes over a label/property pair. Exact
search remains the correctness oracle; ANN surfaces provide HNSW and IVF search
paths for retrieval workloads.

The vector stack includes:

- flat/exact top-k node search and explicit candidate scoring;
- HNSW and IVF approximate indexes;
- batch search and batch scoring;
- graph-expanded candidate scoring from root nodes;
- maintained candidate state for recurring candidate sets;
- index stats, memory accounting, and rebuild maintenance APIs.

Text search is native BM25 over string node properties. Maintained text indexes
are durable registrations with rebuildable in-memory postings. The surface
supports global search plus single-query and batched candidate-scoped scoring,
which makes it useful for hybrid retrieval:

- BM25 over a whole label/property corpus;
- BM25 rerank over graph-expanded or vector-derived candidates;
- batched BM25 rerank over per-query candidate sets;
- vector rerank over BM25 roots;
- graph algorithms as priors or rerank features for retrieval experiments.

The intended working set is in-memory, read-heavy, and local-engine sized rather
than cloud service sized. Disk-backed vector indexes are research scope, not a
requirement for the current release line.

## ISO GQL Posture

`selene-db` targets ISO/IEC 39075:2024 minimum conformance plus a curated set of
optional features. The feature register in `selene-core` is the implementation
source of truth for optional feature status.

Important boundaries:

- Mandatory ISO scalar types are `STRING`, `BOOLEAN`, `INT`, and `FLOAT`.
- Open and closed graph modes are supported.
- Default isolation is serializable; the engine implements strict-serializable
  behavior for a single graph instance.
- ISO GQL does not define a wire format, so this repository does not ship one.
- Auth, tenancy, network policy, and deployment topology are embedder concerns.
- Non-standard syntax must be rejected or flagged; implementation-defined
  features live behind native procedures.

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

Useful docs:

- [Getting started](docs/getting-started.md)
- [Embedding guide](docs/embedding-guide.md)
- [GQL reference](docs/gql-reference.md)
- [Architecture](docs/architecture.md)
- [Graph algorithms](docs/graph-algorithms.md)
- [Persistence and recovery](docs/persistence-and-recovery.md)
- [Observability](docs/observability.md)
- [Performance](docs/performance.md)
- [Contributing](docs/contributing.md)
- [Benchmark registry](BENCHMARKS.md)

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
