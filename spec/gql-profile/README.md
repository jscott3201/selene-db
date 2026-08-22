# GQL profile source

`profile.json` is the source of truth for the current feature, extension, and
partial implementation-defined inventories. `schema.json` documents the closed
source format. Rust deserialization and semantic validation are authoritative;
the generator does not fetch the ISO document, network resources, or local
services.

The ordered compatibility lists preserve the existing public runtime slices;
validation requires their membership to match the typed records exactly.

The implementation-defined table is the existing incomplete inventory. Its
completion and semantic audit belong to M01-PR03.

From the repository root:

```bash
cargo run -p selene-db-profile --bin selene-profile -- --write
cargo run -p selene-db-profile --bin selene-profile -- --check
```

`--write` canonicalizes the source and refreshes checked-in Rust and
`registry.md`. `--check` fails when any generated output is stale.
