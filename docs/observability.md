# Observability

Selene emits metrics through the Rust `metrics` facade when the workspace
`metrics` feature is enabled. The engine does not install an exporter or
recorder; embedders choose a recorder that fits their runtime.

When the feature is disabled, the same call sites compile to no-ops.

## Metrics

| Name | Type | Labels | Description |
|---|---|---|---|
| `selene.queries.total` | counter | `statement.kind` | Executed statements. |
| `selene.query.duration_seconds` | histogram | `statement.kind` | Statement execution latency. |
| `selene.commits.total` | counter | none | Successful graph commits. |
| `selene.commit.duration_seconds` | histogram | none | Successful graph commit latency. |
| `selene.wal.appends.total` | counter | none | Successful WAL appends. |
| `selene.snapshots.total` | counter | none | Finalized snapshots. |
| `selene.snapshot.duration_seconds` | histogram | none | Snapshot finalization latency. |
| `selene.recoveries.total` | counter | none | Successful recovery passes. |
| `selene.recovery.duration_seconds` | histogram | none | Recovery pass latency. |
| `selene.cancellations.total` | counter | none | Cooperative cancellation or timeout events surfaced by the executor. |
| `selene.vector.searches.total` | counter | none | Successful vector search procedure calls. |
| `selene.algorithm.runs.total` | counter | none | Successful algorithm procedure calls. |
| `selene.graph.nodes` | gauge | none | Live node count after a successful commit. |
| `selene.graph.edges` | gauge | none | Live edge count after a successful commit. |

`statement.kind` is intentionally low-cardinality. It uses planner-derived
statement classes such as `query`, `mutation`, `catalog`, `call`, `explain`,
`composite`, and transaction-control names. Query text is never used as a
label.

## Exporter Wiring

Enable the feature on the crate graph and install any `metrics` recorder before
executing statements:

```toml
[dependencies]
selene-gql = { version = "1.0.0", features = ["metrics"] }
```

```rust
use selene_core::GraphId;
use selene_gql::{EmptyProcedureRegistry, Session};
use selene_graph::SharedGraph;

fn install_recorder() {
    // Install a Prometheus, OTLP, or test recorder here.
}

install_recorder();

let graph = SharedGraph::new(GraphId::new(1));
let mut session = Session::new(&graph);
session.execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)?;
```

Exporter setup is deliberately outside the engine boundary. For example, an
application can install a Prometheus, OTLP, or test recorder using the standard
`metrics` ecosystem, then call Selene normally.
