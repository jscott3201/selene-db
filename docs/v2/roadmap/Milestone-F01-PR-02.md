---
plan_id: F01-PR02
milestone: F01
initial_status: proposed
---

# F01-PR02 — Remove downstream public-row APIs and close candidate safety

**Milestone:** [F01: Finish graph identity and mixed topology](Milestone-F01.md)
**Dependencies:** [F01-PR01](Milestone-F01-PR-01.md)
**Carries forward:** M04-PR02; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** #1093
**Focused skills:** `rust-api-design`; `rust-test-design`; `rust-review` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Complete the typed-candidate migration across algorithms, GQL and test adapters, then remove repository-public graph storage-row APIs. This is the closure owner for #1093.

## Start from what exists

The live Part 3B forecast names 25 production files, including graph API owners that were missing from an earlier plan. It also names text/vector builtins, Pagerank filtering and test fixtures. All required callers are in scope even when a mechanical caller was omitted from that forecast. Source: S03.

**Observed live entry points:** `crates/selene-graph/src/graph.rs`, `crates/selene-graph/src/lib.rs`, `crates/selene-graph/src/store.rs`, `crates/selene-algorithms/src/projection.rs`, `crates/selene-algorithms/src/projection/csr.rs`, `crates/selene-gql/src/plan/optimize/live_index_catalog.rs`, `crates/selene-gql/src/runtime/scan.rs`, `crates/selene-gql/src/runtime/builtins/retrieval_filter.rs`, `crates/selene-db/src/lib.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Migrate algorithm projections, scan/expand/filter paths, index catalog access and builtin candidate filters to checked candidates or stable-ID resolvers. Inspect every caller before deleting the producing method.
2. Preserve a strictly algorithm-private dense projection index where the algorithm needs array addressing. That coordinate is not a graph storage row, is never publicly convertible to one and is rebuilt from the pinned graph snapshot.
3. Keep facade DatabaseId ingress checks at the facade boundary. Lower crates must not import selene-db merely to validate a database-scoped handle.
4. Delete public RowIndex exports, raw bitmap producers and row↔ID conversion APIs from their graph owners after callers migrate. Remove compatibility aliases, repeated conversion loops and temporary adapters; do not just hide one re-export.
5. Add a facade-only consumer smoke example and run cross-crate behavior checks. Only merge-complete acceptance here marks old M04-PR02 complete and permits #1093 closure.

## Acceptance and concrete regression cases

- [ ] Public compile-fail/API tests reject graph storage-row exports and node/edge candidate mixing.
- [ ] Algorithm results on sparse/deleted IDs match a stable-ID reference fixture before and after compaction.
- [ ] Indexed scan, reachability, vector/text prefilters and ordinary GQL queries return the same rows as unindexed/stable-ID paths.
- [ ] Foreign facade handles fail at ingress even where their lower numeric GraphId/NodeId matches a local value.
- [ ] An external-style crate can build a database, execute an insert/query and observe structured results without depending on lower graph types.
- [ ] No downstream test retains an old row API simply to simplify fixtures; compile all affected crates and their public documentation.

## Validation and performance

Run graph, algorithms, GQL and facade tests plus changed testing helpers. Use the repository row-arithmetic/API checks, separate doctests and the normal workspace PR gate. Inspect raw-row search matches rather than banning every internal use of u32 or bitmap storage.

Compare full facade queries and algorithm projection construction as well as low-level candidate costs. Keep deterministic ID order where promised; do not introduce sorting into every hot loop solely to make an unordered result easier to test.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No new stable-ID semantics, new query grammar, new public dense-index API or benchmark claim based only on successful compilation.

## Bridge/deletion boundary

No candidate raw-row compatibility bridge remains. VectorCandidateSet remains an unbound stable-ID input type, not bridge debt.

## Standards and reviewer focus

§4.3.1 and §4.4.4. Product boundary: stable IDs versus snapshot-local physical positions.

**Independent review question:** Does the final public surface actually remove the unsafe coordinate system, and do consumers retain good asymptotic behavior?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
