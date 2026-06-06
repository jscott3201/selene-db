# selene-db

`selene-db` is an embeddable Rust property-graph engine built around
ISO/IEC 39075:2024 GQL, native graph algorithms, first-class dense vectors,
first-class JSON metadata, BM25 full-text search, and local persistence.

It is a library, not a service. There is no bundled database server, wire
protocol, auth layer, cloud control plane, extension loader, or procedure-pack
system. Applications link the crates they need, own their network and security
boundary, and run the engine in process.

The project north star is strict ISO GQL at the language boundary plus
pragmatic native retrieval primitives for graph-heavy and agentic-memory
workloads. Non-standard capabilities are exposed through implementation-defined
values, indexes, and `CALL selene.*` / `CALL algo.*` procedures, not by adding
SQL, Cypher, SPARQL, or ad hoc grammar.

## What Is Here

| Area | Current surface |
|---|---|
| GQL | Parser, analyzer, planner, optimizer, executor, parameter binding, source-string plan cache, feature-status reporting, and ISO-oriented errors. |
| Graph storage | In-memory property graph with stable external IDs, dense internal rows, immutable reader snapshots, typed property indexes, composite indexes, and one mutation funnel. |
| Transactions | Serialized writers, snapshot readers, rollback by non-publication, and provider fanout under the write lock. |
| Persistence | WAL, snapshots, MANIFEST-led recovery, retention pruning, audit-log primitives, and rebuildable provider state. |
| Procedures | Native `BuiltinProcedureRegistry` for health/feature reporting, index DDL, vector search/scoring, JSON candidate production, BM25 search/scoring, maintenance, and `algo.*` graph algorithms. |
| Algorithms | Structural, pathfinding, centrality, and community algorithms through `selene-algorithms` and GQL `CALL algo.*` adapters. |
| Vectors | `Value::Vector`, exact scoring/search, HNSW and IVF indexes, batch scoring, graph-expanded scoring, maintained candidate state, index stats, and rebuild APIs. |
| JSON | `Value::Json`, typed GQL properties/parameters/casts, bounded selectors, containment predicates, and exact containment candidate search. |
| Text | BM25 exact scan, maintained text indexes, candidate-scoped scoring, batched scoring, graph/state-expanded scoring, and text-index stats. |
| Safety | Workspace-wide `#![forbid(unsafe_code)]`, `missing_docs = "deny"`, rustls-only dependency posture, file-size checks, secret scans, license checks, fuzz targets, and benchmark hygiene guards. |

## Workspace Crates

There is no umbrella facade crate. Use the layers directly.

| Crate | Owns |
|---|---|
| [`selene-core`](crates/selene-core) | Foundation types: `Value`, `VectorValue`, `JsonValue`, IDs, `DbString`, labels, property maps, schema metadata, codecs, origins, changesets, and core vector kernels. |
| [`selene-graph`](crates/selene-graph) | Graph storage, transactions, mutation funnel, property/composite/vector/text indexes, exact and ANN vector search, BM25 search, exact JSON containment search, maintained candidate state, graph type validation, compaction, and recovery providers. |
| [`selene-persist`](crates/selene-persist) | WAL, snapshots, MANIFEST recovery, retention pruning, and audit log files. It stays below graph semantics. |
| [`selene-algorithms`](crates/selene-algorithms) | Native graph algorithms, projection catalogs, free functions, and the `GraphAlgorithms` convenience trait. |
| [`selene-gql`](crates/selene-gql) | GQL grammar, AST, analysis, planning, optimization, execution, procedure traits, and the built-in procedure registry. |
| [`selene-testing`](crates/selene-testing) | Test fixtures, graph generators, benchmark corpora, opt-in local/remote embedding helpers, and snapshot-harness utilities. |

The intended dependency direction is:

```text
selene-core -> selene-graph -> selene-algorithms -> selene-gql
```

`selene-persist` depends on `selene-core` and remains graph-blind.
`selene-testing` is for tests and benchmarks.

## Quickstart

The crates are not published to crates.io yet. Depend on them by path from a
neighboring application:

```toml
[dependencies]
selene-core = { path = "../selene-db/crates/selene-core" }
selene-graph = { path = "../selene-db/crates/selene-graph" }
selene-gql = { path = "../selene-db/crates/selene-gql" }
```

Create a graph, write through the mutation funnel, and query with GQL:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, db_string};
use selene_gql::{BuiltinProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = SharedGraph::new(GraphId::new(1));

    let person = db_string("Person")?;
    let name = db_string("name")?;

    let mut tx = graph.begin_write();
    {
        let props = PropertyMap::from_pairs([(
            name,
            Value::String(db_string("Ada")?),
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

For a fuller walk through graph creation, typed properties, parameters,
transactions, and persistence wiring, start with
[Getting Started](docs/getting-started.md) and the
[Embedding Guide](docs/embedding-guide.md).

## GQL Boundary

GQL is the only query and mutation language. The parser rejects grammar drift
instead of accepting other languages as aliases.

Examples of supported ISO-oriented surface:

```gql
MATCH (p:Person)-[:KNOWS]->(friend)
WHERE p.name = 'Ada'
RETURN friend.name AS name
ORDER BY name
LIMIT 10
```

```gql
INSERT (:Person {name: 'Grace'})
FINISH
```

```gql
CALL selene.feature_status()
YIELD feature_id, feature_name, status, rationale
RETURN feature_id, status
```

Native procedures are part of the engine, not loadable extensions. Tests can
inject alternate `ProcedureRegistry` implementations, but production code uses
the in-tree `BuiltinProcedureRegistry`.

## Native Retrieval

The retrieval stack is deliberately composable. selene-db provides graph,
vector, text, and algorithm primitives; embedders decide the memory or search
policy that fits their application.

### Vectors

Vectors are first-class values:

- node properties can store `Value::Vector(VectorValue)`;
- components are finite `f32` values behind shared storage;
- supported metrics are `squared_euclidean`, `cosine`, and
  `negative_inner_product`;
- exact scoring is the correctness oracle;
- HNSW and IVF indexes provide in-memory ANN paths;
- graph-expanded and maintained-state scorers compose topology with vectors;
- rebuild and stats procedures make derived index state observable.

Representative GQL calls:

```gql
CALL selene.create_vector_index('Document', 'embedding', 1536, 'hnsw', NULL, 'cosine')
```

```gql
CALL selene.vector_search_nodes_ann(
  'Document', 'embedding', $query, 10, 'cosine', 64
)
YIELD node_id, distance
RETURN node_id, distance
```

```gql
CALL selene.vector_score_candidate_state_expanded_batch(
  'embedding', $queries, 'current_support_facts',
  $root_sets, 'SUPPORTS', 10, 'intersection', 'outgoing', 'cosine'
)
YIELD query_index, node_id, distance
RETURN query_index, node_id, distance
```

### BM25 Text

BM25 is native over string node properties:

- exact scan remains the small-corpus oracle;
- maintained text indexes keep rebuildable postings for repeated queries;
- candidate-scoped scoring lets graph-produced node sets feed BM25 directly;
- batch and maintained-state-expanded procedures support multi-query
  retrieval rows.

Representative GQL calls:

```gql
CALL selene.create_text_index('Document', 'body')
```

```gql
CALL selene.text_search_nodes('Document', 'body', $query, 10)
YIELD node_id, score
RETURN node_id, score
```

```gql
CALL selene.text_score_nodes_batch('Document', 'body', $queries, $candidates, 10)
YIELD query_index, node_id, score
RETURN query_index, node_id, score
```

### JSON Metadata

JSON is a first-class value for structured agent payloads and metadata:

- node properties can store `Value::Json(JsonValue)`;
- `JSON` participates in GQL typed parameters, typed predicates, and casts;
- closed graph schemas can declare canonical JSON defaults with a string literal
  such as `payload :: JSON DEFAULT '{"kind":"episodic"}'`;
- scalar functions provide parse/stringify/type, array/object shape
  introspection, bounded path selectors, existence checks, and recursive
  containment;
- `selene.json_contains_nodes` turns JSON metadata predicates into graph node
  candidates for vector/text reranking.

Representative GQL call:

```gql
CALL selene.json_contains_nodes(
  'Document', 'payload', json('{"memory":{"kind":"episodic"}}'), 10
)
YIELD node_id
RETURN node_id
```

### Graph Algorithms

`selene-algorithms` owns native algorithms over immutable projections. GQL
exposes them through `CALL algo.*`, including PageRank, shortest paths,
weakly connected components, label propagation, Louvain, triangle count, and
related structural primitives.

```gql
CALL algo.pagerank('projection_name', 0.85, 100, 1e-6, NULL)
YIELD node_id, score
RETURN node_id, score
ORDER BY score DESC
LIMIT 20
```

## Persistence

Persistence stays below graph semantics:

- WAL records changes and provider sections;
- snapshots store graph and provider state;
- MANIFEST recovery chooses the live snapshot/WAL set;
- retention pruning removes old snapshots and WAL archives;
- vector and text providers rebuild derived in-memory state from primary graph
  values during recovery and compaction.

The library crates are allocator-agnostic. Benchmark binaries use mimalloc by
default so allocator A/B rows can be measured without forcing an embedder-wide
allocator policy.

## ISO GQL Posture

`selene-db` targets ISO/IEC 39075:2024 minimum conformance plus a curated set of
optional features. `selene-core::feature_register` is the source of truth for
optional feature status.

Important boundaries:

- vectors are implementation-defined values, not grammar extensions;
- open and closed graph modes are supported;
- default isolation is serializable for one graph instance;
- ISO GQL does not define a wire format, so this repository does not ship one;
- auth, tenancy, network policy, and deployment topology are embedder concerns.

## Development

The workspace uses a pinned stable Rust toolchain. Read
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
secret scan, benchmark-doc checks, and whitespace checks should stay green.

Install local hooks once per clone:

```bash
scripts/install-hooks.sh
```

## Benchmarks

Use the serialized runner. Do not run `cargo bench --workspace`; Cargo can run
bench binaries concurrently and pollute wall-clock medians.

```bash
scripts/run-benches.sh --list
scripts/run-benches.sh --smoke
scripts/run-benches.sh --profile quick --bench vector_graph_retrieval
scripts/run-benches.sh --profile quick --bench procedure_call_repeat --filter vector
scripts/run-benches.sh --profile quick --bench text_search_bm25
scripts/run-benches.sh --bench vector_index_rebuild --allocator system
```

Local embedding rows are opt-in and require an ignored `.env` file plus either
a local OpenAI-compatible oMLX endpoint or an OpenRouter embedding key:

```bash
set -a; source .env; set +a
SELENE_EMBEDDING_BENCH=1 \
SELENE_EMBEDDING_CORPUS=scaled_ambiguous_memory \
SELENE_GRAPH_HINT_DOCS_PER_TOPIC=2 \
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
|---|---|
| Linux x86_64 / aarch64 | Primary deployment target. |
| macOS Apple Silicon / Intel | Primary development target. |
| Windows | Out of scope for now. |

## License

Dual-licensed under MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE), [NOTICE](NOTICE), and
[THIRDPARTY.md](THIRDPARTY.md).
