# Getting started

This guide walks you from an empty Cargo project to running your first ISO GQL query against `selene-db` in under ten minutes. It assumes you are comfortable with `cargo` and basic Rust.

By the end you will have:

1. Added `selene-db` to a Cargo project as path dependencies.
2. Built a graph, added nodes and edges, and read them back with a `MATCH` query.
3. Used labels, typed properties, and parameter bindings in a query pipeline.
4. Confirmed read-your-writes durability across multiple transactions, and seen where on-disk persistence lives.

If you only want the 30-second version, skip to the [Hello-graph](#example-1-hello-graph) example.

---

## Prerequisites

- **Rust 1.95.0 or later** (workspace `rust-version`). `rustup show` will confirm; `rustup update stable` if you are behind.
- **Edition 2024**. New projects you generate with `cargo new` on modern toolchains already use this; the example `Cargo.toml` snippets below set it explicitly.
- **macOS or Linux**. Windows is out of scope for v1.0.

You do not need a database server, a container, or a wire client. `selene-db` is an in-process library: it runs in the same address space as your Rust application.

---

## Adding selene-db to a Cargo project

`selene-db` is not yet published to crates.io. Embedders depend on the workspace crates by path:

```bash
git clone https://github.com/Aionforge-Labs/selene-db.git
cargo new my-graph-app
cd my-graph-app
```

In `my-graph-app/Cargo.toml`, add the crates you need. For a first query you only need three:

```toml
[package]
name = "my-graph-app"
version = "0.1.0"
edition = "2024"

[dependencies]
selene-core  = { path = "../selene-db/crates/selene-core" }
selene-graph = { path = "../selene-db/crates/selene-graph" }
selene-gql   = { path = "../selene-db/crates/selene-gql" }
```

For on-disk persistence, add `selene-persist`. For graph algorithms — PageRank, betweenness, Louvain, and the rest, reachable both as a native Rust API and via `CALL algo.*` — add `selene-algorithms`:

```toml
selene-persist    = { path = "../selene-db/crates/selene-persist" }
selene-algorithms = { path = "../selene-db/crates/selene-algorithms" }
```

Run `cargo build` once to confirm the dependency graph resolves before moving on.

---

## Example 1: Hello-graph

This program creates a graph in memory, inserts three `Person` nodes connected by two `KNOWS` edges, then runs a `MATCH` query and prints the names.

Replace `src/main.rs` with:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, db_string};
use selene_graph::SharedGraph;
use selene_gql::{
    EmptyProcedureRegistry, Session, StatementOutput, analyze, execute_statement, parse, plan,
};

fn main() {
    // Step 1: open a new in-memory graph.
    let graph = SharedGraph::new(GraphId::new(1));

    // Step 2: construct the label and property keys.
    let person = db_string("Person").unwrap();
    let knows = db_string("KNOWS").unwrap();
    let name = db_string("name").unwrap();

    // Step 3: open a write transaction. Writes are serialized through a
    // single graph-wide write lock; readers see lock-free immutable snapshots.
    let mut tx = graph.begin_write();
    {
        let mut mutator = tx.mutator();

        let mut ada_props = PropertyMap::new();
        ada_props
            .set(name.clone(), Value::String(db_string("Ada").unwrap()))
            .unwrap();
        let ada = mutator
            .create_node(LabelSet::single(person.clone()), ada_props)
            .unwrap();

        let mut grace_props = PropertyMap::new();
        grace_props
            .set(name.clone(), Value::String(db_string("Grace").unwrap()))
            .unwrap();
        let grace = mutator
            .create_node(LabelSet::single(person.clone()), grace_props)
            .unwrap();

        let mut linus_props = PropertyMap::new();
        linus_props
            .set(name, Value::String(db_string("Linus").unwrap()))
            .unwrap();
        let linus = mutator
            .create_node(LabelSet::single(person), linus_props)
            .unwrap();

        mutator
            .create_edge(knows.clone(), ada, grace, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(knows, grace, linus, PropertyMap::new())
            .unwrap();
    }
    tx.commit().unwrap();

    // Step 4: run a GQL MATCH against the committed snapshot.
    let registry = EmptyProcedureRegistry;
    let statement = parse("MATCH (p:Person) RETURN p.name").unwrap();
    let analyzed = analyze(statement, &registry, None).unwrap();
    let planned = plan(&analyzed, &registry).unwrap();

    let mut session = Session::new(&graph);
    let output = execute_statement(&planned, &mut session, &registry).unwrap();

    let StatementOutput::Rows(rows) = output else {
        panic!("MATCH ... RETURN should yield rows");
    };

    println!("found {} person rows", rows.row_count());
    for row in rows.rows() {
        if let Some(Value::String(s)) = row.values().first() {
            println!("  - {}", s.as_str());
        }
    }
}
```

Run it:

```bash
cargo run
```

Expected output (row order is not specified by ISO GQL without an `ORDER BY`):

```text
found 3 person rows
  - Ada
  - Grace
  - Linus
```

### What just happened

- `SharedGraph::new` allocates a fresh graph with a stable `GraphId`. The graph is empty, has no schema bound, and accepts ad-hoc labels and properties (an **open** graph per ISO 39075 GG01).
- `graph.begin_write()` takes the write lock and returns a `WriteTxn`. All mutation goes through `tx.mutator()`, which validates and accumulates `Change` records.
- `tx.commit()` atomically publishes a new immutable snapshot via `ArcSwap`. Failure rolls everything back; partial commits never become visible.
- `parse` → `analyze` → `plan` → `execute_statement` is the canonical pipeline. The intermediate `AnalyzedStatement` and `ExecutionPlan` are cacheable across executions of the same query text against the same schema.
- `Session::new(&graph)` binds the executor to the current snapshot. Long-lived sessions can be reused across statements.

---

## Example 2: Labels, typed properties, and pipelines

This example adds typed properties (`Int`, `Bool`), uses multiple labels, and chains GQL clauses with `WHERE`, `ORDER BY`, and `LIMIT`. It demonstrates the shape you reach for once "open graph, ad-hoc keys" is no longer enough.

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, db_string};
use selene_graph::SharedGraph;
use selene_gql::{
    EmptyProcedureRegistry, Session, StatementOutput, analyze, execute_statement, parse, plan,
};

fn main() {
    let graph = SharedGraph::new(GraphId::new(1));

    let person = db_string("Person").unwrap();
    let engineer = db_string("Engineer").unwrap();
    let name = db_string("name").unwrap();
    let age = db_string("age").unwrap();
    let active = db_string("active").unwrap();

    // Build a LabelSet with two labels for one node.
    // LabelSet::insert returns `bool` (true if the label was newly added);
    // the workspace lints deny `unused_must_use`, so bind or discard explicitly.
    let mut engineer_labels = LabelSet::new();
    let _ = engineer_labels.insert(person.clone());
    let _ = engineer_labels.insert(engineer);

    let mut tx = graph.begin_write();
    {
        let mut mutator = tx.mutator();

        let mut p1 = PropertyMap::new();
        p1.set(name.clone(), Value::String(db_string("Ada").unwrap())).unwrap();
        p1.set(age.clone(), Value::Int(36)).unwrap();
        p1.set(active.clone(), Value::Bool(true)).unwrap();
        mutator
            .create_node(engineer_labels.clone(), p1)
            .unwrap();

        let mut p2 = PropertyMap::new();
        p2.set(name.clone(), Value::String(db_string("Grace").unwrap()))
            .unwrap();
        p2.set(age.clone(), Value::Int(85)).unwrap();
        p2.set(active.clone(), Value::Bool(true)).unwrap();
        mutator
            .create_node(engineer_labels.clone(), p2)
            .unwrap();

        let mut p3 = PropertyMap::new();
        p3.set(name, Value::String(db_string("Bob").unwrap())).unwrap();
        p3.set(age, Value::Int(22)).unwrap();
        p3.set(active, Value::Bool(false)).unwrap();
        mutator
            .create_node(LabelSet::single(person), p3)
            .unwrap();
    }
    tx.commit().unwrap();

    // Active engineers, oldest first, top 2.
    let registry = EmptyProcedureRegistry;
    let query = r#"
        MATCH (p:Person & Engineer)
        WHERE p.active = true
        RETURN p.name AS name, p.age AS age
        ORDER BY p.age DESC
        LIMIT 2
    "#;

    let statement = parse(query).unwrap();
    let analyzed = analyze(statement, &registry, None).unwrap();
    let planned = plan(&analyzed, &registry).unwrap();

    let mut session = Session::new(&graph);
    let StatementOutput::Rows(rows) = execute_statement(&planned, &mut session, &registry).unwrap()
    else {
        panic!("expected row output");
    };

    for row in rows.rows() {
        let values = row.values();
        let name_str = match &values[0] {
            Value::String(s) => s.as_str().to_string(),
            other => format!("{other:?}"),
        };
        let age_num = match values[1] {
            Value::Int(n) => n,
            _ => panic!("age should be Int"),
        };
        println!("{name_str} ({age_num})");
    }
}
```

Expected output:

```text
Grace (85)
Ada (36)
```

### Notes

- `LabelSet::new()` plus `insert` lets a node carry multiple labels. Use `LabelSet::single(label)` when you only need one.
- `PropertyMap::set` accepts any `Value` variant. The full type list is in [`selene-core/src/value.rs`](../crates/selene-core/src/value.rs); the mandatory ISO types `STRING`, `BOOLEAN`, `INT`, `FLOAT` correspond to `Value::String`, `Value::Bool`, `Value::Int`, `Value::Float`.
- `(p:Person & Engineer)` requires both labels. `(p:Person | Engineer)` requires at least one. The GQL Flagger rejects label-expression forms outside the optional features selene-db claims; see [the GQL reference](gql-reference.md) for the full surface.
- `DbString::as_str()` returns the string slice for labels, property keys, and
  `Value::String` payloads. `db_string(...)` applies the IL013 per-string byte
  limit before constructing the owned database string.
- For literal parameter binding (`$name`, `$age`, etc.), pass values through `Session` rather than baking them into the query text. The full parameter API is covered in [the embedding guide](embedding-guide.md).

---

## Example 3: Multi-transaction durability and where persistence lives

The graph is in-memory by default. Each committed transaction publishes a new snapshot that subsequent readers and writers observe immediately. This example proves that contract end-to-end:

```rust
use selene_core::{GraphId, LabelSet, PropertyMap, Value, db_string};
use selene_graph::SharedGraph;
use selene_gql::{
    EmptyProcedureRegistry, Session, StatementOutput, analyze, execute_statement, parse, plan,
};

fn count_persons(graph: &SharedGraph) -> usize {
    let registry = EmptyProcedureRegistry;
    let statement = parse("MATCH (p:Person) RETURN p.name").unwrap();
    let analyzed = analyze(statement, &registry, None).unwrap();
    let planned = plan(&analyzed, &registry).unwrap();
    let mut session = Session::new(graph);
    let StatementOutput::Rows(rows) = execute_statement(&planned, &mut session, &registry).unwrap()
    else {
        return 0;
    };
    rows.row_count()
}

fn insert_person(graph: &SharedGraph, who: &str) {
    let person = db_string("Person").unwrap();
    let name = db_string("name").unwrap();

    let mut tx = graph.begin_write();
    let mut props = PropertyMap::new();
    props
        .set(name, Value::String(db_string(who).unwrap()))
        .unwrap();
    tx.mutator()
        .create_node(LabelSet::single(person), props)
        .unwrap();
    tx.commit().unwrap();
}

fn main() {
    let graph = SharedGraph::new(GraphId::new(1));

    assert_eq!(count_persons(&graph), 0);

    insert_person(&graph, "Ada");
    assert_eq!(count_persons(&graph), 1);

    insert_person(&graph, "Grace");
    assert_eq!(count_persons(&graph), 2);

    println!("two commits, two reads, both saw the new state");
}
```

That demonstrates **read-your-writes** across transactions on a single graph handle: commit publishes a new snapshot atomically, the next query observes it.

### On-disk persistence

`SharedGraph::new` does not write to disk. To persist a graph across process restarts, you wire the `selene-persist` crate's WAL and snapshot pipeline against the graph's `CoreProvider`. The shape is:

- **Write side**: every `tx.commit()` emits a `Vec<Change>`; the embedder pipes those into `selene_persist::WalWriter::open(path, WalConfig::default())` (WAL file, `SLDB` magic).
- **Snapshot side**: periodically, the embedder dumps the current graph to a `SnapshotBuilder` (snapshot file, `SLSN` magic) and truncates the WAL.
- **Recovery on restart**: `selene_persist::recover(dir, &registry)` reads the latest snapshot, replays the WAL tail, and routes both into the graph's `CoreProvider` (the engine's `IndexProvider`/`DurableProvider`/`RecoveryProvider` plumbing).

This wiring is intentionally explicit so embedders can choose their own commit-to-disk policy (sync every commit, batch, flush on idle, etc.) and own their snapshot cadence. The full worked example, including `ProviderRegistry` setup and a process-restart test, lives in [docs/persistence-and-recovery.md](persistence-and-recovery.md).

---

## Common patterns

- **Re-using parsed plans**: `parse`, `analyze`, and `plan` are pure functions of (query text, registry, schema). Cache the resulting `ExecutionPlan` and re-run it through `execute_statement` with fresh `Session`s when the query text is fixed.
- **One graph per logical database**: `SharedGraph` is `Clone`-cheap (it wraps an `Arc`). Pass it across threads; reads are lock-free.
- **Avoiding panics**: every example above uses `.unwrap()` for readability. Production code should pattern-match `GraphError`, `AnalysisError`, `PlannerError`, and `ExecutorError`. Each is a typed `thiserror::Error` with miette-friendly diagnostics.
- **Database strings**: construct labels, property keys, aliases, and string
  values with `db_string(...)`. Clone existing `DbString` keys when a mutation
  API consumes the key and you need to reuse it later.
- **Mutations from GQL**: examples 1-3 mutate through `Mutator` for clarity. The same effects are available through ISO GQL `INSERT`, `SET`, and `DELETE` clauses; pick the surface that fits your application.

---

## Where to next

- [Embedding guide](embedding-guide.md) — long-running embedders, parameter binding, error handling, session reuse.
- [GQL reference](gql-reference.md) — the ISO GQL surface selene-db supports, including which optional features are claimed.
- [Architecture](architecture.md) — crate boundaries, threading model, snapshot semantics.
- [Graph algorithms](graph-algorithms.md) — the native `selene-algorithms` API and `CALL algo.*` for PageRank, betweenness, Louvain, and the rest.
- [Persistence and recovery](persistence-and-recovery.md) — WAL, snapshots, and the recovery flow.
- [Performance](performance.md) — benchmarks, tuning knobs, and what numbers to expect.
