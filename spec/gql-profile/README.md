# GQL profile source

`profile.json` is the only source of truth for the target profile, feature
implications, runtime inventory, claim state, extensions, and the partial
implementation-defined inventory. `schema.json` documents the closed source
format. Rust deserialization and semantic validation are authoritative; the
generator does not fetch the ISO document, network resources, or local services.

The direct target selection is separate from runtime support and formal claim
state. Validation closes the selection over the imported Table 10 edges and
rejects inconsistent runtime support, claims, evidence, and release status.
The ordered compatibility lists preserve the surviving public runtime order.

The imported 71 edges cover only the all-of relationships in Clause 24.7,
Table 10. Alternative-choice requirements and broader conformance rules belong
to M01-PR05 and are not represented as implication edges.

The implementation-defined table is the existing incomplete inventory. Its
completion and semantic audit belong to M01-PR03.

From the repository root:

```bash
cargo run -p selene-db-profile --bin selene-profile -- --write
cargo run -p selene-db-profile --bin selene-profile -- --check
```

`--write` canonicalizes the source and refreshes checked-in Rust and
`registry.md`, generated Rust data, and
`docs/gql/conformance/features.md`. `--check` fails when any generated output is
stale.
