# GQL profile source

`profile.json` is the only source of truth for the target profile, feature
implications, runtime inventory, claim state, extensions, and Selene's Annex B
decisions. `schema.json` documents the closed source format. Rust
deserialization and semantic validation are authoritative; the generator does
not fetch the ISO document, network resources, or local services.

The direct target selection is separate from runtime support and formal claim
state. Validation closes the selection over the imported Table 10 edges and
rejects inconsistent runtime support, claims, evidence, and release status.
The ordered compatibility lists preserve the surviving public runtime order.

The imported 71 edges cover only the all-of relationships in Clause 24.7,
Table 10. Alternative-choice requirements and broader conformance rules belong
to M01-PR05 and are not represented as implication edges.

The hand-authored Rust inventory oracle contains only the 117 official Annex B
IDs and category boundaries. Validation requires exactly that inventory and
checks each record's applicability, typed disposition, clause and evidence
references, and pending owner. The oracle does not duplicate Selene decisions.
`release_claimable` remains false while any applicable decision is pending and
until the later conformance work is complete.

From the repository root:

```bash
cargo run -p selene-db-profile --bin selene-profile -- --write
cargo run -p selene-db-profile --bin selene-profile -- --check
```

`--write` canonicalizes the source and refreshes `registry.md`, generated Rust
data, `docs/gql/conformance/features.md`, and
`docs/gql/conformance/implementation-defined.md`. `--check` fails when any
generated output is stale.
