# Getting started

This guide creates an in-memory catalog graph and runs GQL through the stable
`selene-db` facade. No database server or wire client is required.

## Prerequisites

- Rust 1.97.1 or later
- Rust edition 2024
- macOS or Linux

## Add the facade

The examples use the current source coordinate. Verify that the alpha is
published before resolving it from crates.io; otherwise use a path dependency
to a source checkout.

Clone the canonical repository when using a source checkout, then point the
single `selene-db` dependency at `<checkout>/crates/selene-db`:

```bash
git clone https://github.com/jscott3201/selene-db.git
```

```toml
[package]
name = "my-graph-app"
version = "0.1.0"
edition = "2024"

[dependencies]
selene-db = { version = "2.0.0-alpha.1" }
```

Applications should import only `selene-db`. Lower workspace crates are
advanced engine APIs and do not carry the facade's 2.x stability promise.

## Create and select a graph

`DatabaseBuilder::build` creates an empty in-memory catalog. Create a schema and
graph through `Catalog`, then select that graph when constructing a session.

```rust
use selene_db::{CreatePolicy, Database, ExecutionOutcome, ObjectPath, SchemaPath};

fn main() -> Result<(), selene_db::Error> {
    let database = Database::builder().build();
    let catalog = database.catalog();

    let schema = SchemaPath::regular("selene", "memory")?;
    catalog.create_schema(&schema, CreatePolicy::Strict)?;

    let graph = ObjectPath::regular("selene", "memory", "people")?;
    catalog.create_graph(&graph, None, CreatePolicy::Strict)?;

    let session = database.session(&graph)?;
    session.execute(
        "INSERT (:Person {name: 'Ada'}), (:Person {name: 'Grace'}) FINISH",
    )?;

    let outcome = session.execute(
        "MATCH (p:Person) RETURN p.name AS name ORDER BY name",
    )?;
    assert_eq!(outcome, ExecutionOutcome::Rows { row_count: 2 });

    Ok(())
}
```

Run the program with `cargo run`.

The current facade reports execution summaries, not row values. A row-producing
statement returns `ExecutionOutcome::Rows { row_count }`; a write without rows
returns a write summary.

## Catalog DDL through GQL

A selected session resolves relative graph and graph-type references against
its graph's schema. Absolute references retain the `/schema/object` GQL form.

```rust
# use selene_db::{CreatePolicy, Database, ObjectPath, SchemaPath};
# fn example() -> Result<(), selene_db::Error> {
# let database = Database::builder().build();
# let catalog = database.catalog();
# let schema = SchemaPath::regular("selene", "memory")?;
# catalog.create_schema(&schema, CreatePolicy::Strict)?;
# let graph = ObjectPath::regular("selene", "memory", "people")?;
# catalog.create_graph(&graph, None, CreatePolicy::Strict)?;
let session = database.session(&graph)?;
session.execute("CREATE GRAPH IF NOT EXISTS archive ANY")?;

let archive = ObjectPath::regular("selene", "memory", "archive")?;
assert!(database.session(&archive).is_ok());
# Ok(())
# }
```

Each session retains the selected graph's stable catalog identity. Dropping and
recreating the same path makes the old session stale; it never switches to the
replacement.

## Current facade boundaries

The facade session is owned, lifetime-free, and stateless between requests. It
does not yet expose parameters, transactions, session controls, cancellation,
authentication, time-zone state, persistence configuration, or row-value
materialization. Stateful controls return a structured error rather than being
accepted without durable session state.

See the [Embedding Guide](embedding-guide.md) for lifecycle and integration
details, and the [GQL Reference](gql-reference.md) for supported language forms.
