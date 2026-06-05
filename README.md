# selene-db

`selene-db` is an embeddable Rust property graph engine built around
ISO/IEC 39075:2024 GQL, strict in-process transactions, native graph
algorithms, first-class dense vectors, and BM25 text search.

It is library-first. There is no database server, wire protocol, auth layer,
cloud control plane, extension loader, or procedure-pack system in this
repository. Applications link the crates they need, own the network/security
boundary, and run the engine in process.

The north star is ISO GQL conformance plus pragmatic native retrieval features
for local graph and agentic-memory workloads. Non-standard capabilities are
exposed through implementation-defined values, indexes, and `CALL` procedures,
not by extending the GQL grammar.

## Current Shape

| Area | Current engine surface |
| --- | --- |
| GQL | ISO-oriented parser, analyzer, planner, optimizer, executor, parameter binding, source-string plan cache, and feature-status reporting. |
| Storage | In-memory property graph with stable external node/edge IDs, dense internal row indexes, copy-on-write snapshots, Roaring label indexes, typed property indexes, composite indexes, and one mutation funnel. |
| Transactions | One writer per graph, immutable reader snapshots, rollback by non-publication, and provider fanout under the write lock. |
| Persistence | Graph-blind WAL, snapshots, MANIFEST-led recovery, retention pruning, and audit-log primitives. |
| Procedures | A fixed native `BuiltinProcedureRegistry` for platform procedures, vector search/scoring, BM25 search/scoring, index metadata, maintenance, and `algo.*` graph algorithms. |
| Algorithms | Structural, pathfinding, centrality, and community algorithms through `selene-algorithms` plus GQL `CALL algo.*` adapters. |
| Vectors | `Value::Vector`, finite `f32` vector storage, exact search/scoring, HNSW and IVF indexes, batched ANN/exact search, graph-expanded scoring, maintained candidate state, and index rebuild/accounting APIs. |
| Text | Durable BM25 text-index registrations over string node properties, exact scan fallback for global search, maintained postings, candidate-scoped scoring, batched scoring, state-expanded scoring, and text-index stats. |
| Safety | Workspace-wide `#![forbid(unsafe_code)]`, `missing_docs = "deny"`, rustls-only dependency posture, file-size gates, secret scans, license checks, fuzz targets, and benchmark hygiene checks. |

## Workspace Crates

There is no umbrella crate. Use the layers directly.

| Crate | Owns |
| --- | --- |
| [`selene-core`](crates/selene-core) | Foundation types: `Value`, `VectorValue`, IDs, `IStr`, labels, property maps, schema metadata, codecs, origins, and changesets. |
| [`selene-graph`](crates/selene-graph) | Graph storage, transactions, mutation funnel, property/composite/vector/text indexes, exact and ANN vector search, BM25 search, maintained candidate state, graph type validation, compaction, and recovery providers. |
| [`selene-persist`](crates/selene-persist) | WAL, snapshots, MANIFEST recovery, retention pruning, and audit log files. It stays below graph semantics. |
| [`selene-algorithms`](crates/selene-algorithms) | Native graph algorithms, projection catalogs, free functions, and the `GraphAlgorithms` convenience trait. |
| [`selene-gql`](crates/selene-gql) | GQL grammar, AST, analysis, planning, optimization, execution, procedure traits, and the built-in registry. |
| [`selene-testing`](crates/selene-testing) | Test fixtures, graph generators, benchmark corpora, local oMLX embedding helpers, and snapshot-harness utilities. |

The intended dependency direction is narrow:

```text
selene-core -> selene-graph -> selene-algorithms -> selene-gql
```

`selene-persist` depends on `selene-core` and remains graph-blind.
`selene-testing` is for tests and benchmarks.

## Quickstart

While the repository is private and unpublished, depend on crates by path:

```toml
[dependencies]
selene-core = { path = "../selene-db/crates/selene-core" }
selene-graph = { path = "../selene-db/crates/selene-graph" }
selene-gql = { path = "../selene-db/crates/selene-gql" }
```

Create a graph, write through the mutation funnel, and query with GQL:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, intern};
use selene_gql::{BuiltinProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = SharedGraph::new(GraphId::new(1));

    let person = intern("Person")?;
    let name = intern("name")?;

    let mut tx = graph.begin_write();
    {
        let props = PropertyMap::from_pairs([(
            name,
            Value::String(intern("Ada")?),
        )])?;
        tx.mutator().create_node(LabelSet::single(person), props)?;
    }
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
persistence wiring, start with [Getting Started](docs/getting-started.md) and
the [Embedding Guide](docs/embedding-guide.md).

## GQL And Procedures

GQL is the only query and mutation language. The parser rejects SQL, Cypher,
SPARQL, and other grammar drift instead of accepting them as aliases.

Native engine features outside mandatory ISO syntax use ISO `CALL`:

```gql
CALL selene.health()
CALL selene.feature_status()
CALL selene.verify()

CALL selene.create_index(...)
CALL selene.create_vector_index(...)
CALL selene.create_text_index(...)
CALL selene.vector_index_stats()
CALL selene.text_index_stats()

CALL selene.vector_search_nodes(...)
CALL selene.vector_search_nodes_ann(...)
CALL selene.vector_score_nodes(...)
CALL selene.vector_score_expanded_candidates(...)
CALL selene.vector_score_candidate_state_expanded_batch(...)
CALL selene.vector_candidate_states()

CALL selene.text_search_nodes(...)
CALL selene.text_score_nodes(...)
CALL selene.text_score_nodes_batch(...)
CALL selene.text_score_candidate_state_expanded_batch(...)

CALL algo.pagerank(...)
CALL algo.shortest_path(...)
CALL algo.weakly_connected_components(...)
```

`BuiltinProcedureRegistry` is native engine code fixed at construction time. It
is not a third-party extension mechanism. Tests can still inject alternate
`ProcedureRegistry` implementations where analyzer or executor behavior needs an
artificial procedure surface.

## Retrieval Stack

Vectors are first-class values. A node property can hold `Value::Vector`, and
the graph layer can maintain vector indexes over `(label, property)`:

- flat exact scan;
- HNSW approximate search;
- IVF approximate search;
- exact scoring over explicit candidate sets;
- batched exact and ANN search;
- graph-expanded candidate scoring from root nodes;
- maintained candidate state for recurring graph-derived candidate sets;
- index stats, memory accounting, rebuild, and recommended rebuild procedures.

Text search is native BM25 over string node properties. A maintained text index
is a durable graph registration with rebuildable in-memory postings. The text
surface supports:

- global BM25 search by label/property;
- exact candidate-scoped BM25 scoring;
- batched candidate-scoped BM25 scoring;
- maintained-state plus graph-expanded BM25 batch scoring;
- text-index stats, create, and drop procedures.

Graph algorithms are retrieval primitives too. The current product-shaped
benchmark direction is to compose graph-derived roots and candidate state with
vector and BM25 scoring rather than hard-code one memory policy into the engine.

## Persistence

Persistence is graph-blind below the provider boundary:

- WAL records graph changes and provider sections.
- Snapshots store graph and provider state.
- MANIFEST recovery selects the live snapshot/WAL set.
- Retention pruning removes old snapshots and WAL archives.
- Providers rebuild derived vector/text/index state from primary graph values
  during recovery and compaction.

The public crates are allocator-agnostic. Benchmark binaries use mimalloc by
default so allocator A/B rows can be measured without changing library behavior.

## ISO GQL Posture

`selene-db` targets ISO/IEC 39075:2024 minimum conformance plus a curated set of
optional features. The feature register in `selene-core` is the
implementation-visible source of truth for optional feature status.

Important boundaries:

- Mandatory scalar types currently include `STRING`, `BOOLEAN`, `INT`, and
  `FLOAT`.
- Vectors are implementation-defined values, not ISO grammar extensions.
- Open and closed graph modes are supported.
- Default isolation is serializable for a single graph instance.
- ISO GQL does not define a wire format, so this repository does not ship one.
- Auth, tenancy, network policy, and deployment topology are embedder concerns.

## Development

The workspace uses a pinned stable Rust toolchain; read
[`rust-toolchain.toml`](rust-toolchain.toml) for the active version.

Common validation for code changes:

```bash
cargo fmt --all --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --all-features --profile default
cargo test --workspace --locked --all-features --doc
cargo doc --workspace --no-deps --locked
cargo deny check bans licenses sources
cargo audit -d /private/tmp/selene-advisory-db
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
bash .github/scripts/check-thirdparty-current.sh
bash .github/scripts/check-no-rowid-arith.sh
bash .github/scripts/check-no-version-locked-feature-error.sh
bash .github/scripts/check-bench-invocation.sh
bash .github/scripts/check-benchmarks-doc.sh .
git diff --check
```

Docs-only changes can use the relevant subset, but formatting, file-size,
secret scan, benchmark-doc checks, and `git diff --check` should stay cheap and
green.

Install local hooks once per clone:

```bash
scripts/install-hooks.sh
```

## Benchmarks

Benchmarks run through the serialized helper. Do not run
`cargo bench --workspace`; Cargo can execute bench binaries concurrently and
pollute wall-clock medians.

Useful commands:

```bash
scripts/run-benches.sh --list
scripts/run-benches.sh --smoke
scripts/run-benches.sh --profile quick --bench vector_graph_retrieval
scripts/run-benches.sh --profile quick --bench procedure_call_repeat --filter vector
scripts/run-benches.sh --profile quick --bench text_search_bm25
scripts/run-benches.sh --bench vector_index_rebuild --allocator system
```

Local oMLX embedding rows are opt-in and require the developer's local
OpenAI-compatible embedding endpoint plus ignored `.env` credentials:

```bash
set -a; source .env; set +a
SELENE_OMLX_EMBEDDING_BENCH=1 \
SELENE_OMLX_CORPUS=scaled_ambiguous_memory \
SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC=2 \
scripts/run-benches.sh --profile quick --bench procedure_call_repeat --filter query_root
```

See [BENCHMARKS.md](BENCHMARKS.md) for registered bench targets, commands, and
current local evidence.

## Project Docs

- [Getting Started](docs/getting-started.md)
- [Embedding Guide](docs/embedding-guide.md)
- [GQL Reference](docs/gql-reference.md)
- [Architecture](docs/architecture.md)
- [Graph Algorithms](docs/graph-algorithms.md)
- [Persistence And Recovery](docs/persistence-and-recovery.md)
- [Observability](docs/observability.md)
- [Performance](docs/performance.md)
- [Contributing](docs/contributing.md)
- [Benchmark Registry](BENCHMARKS.md)

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
