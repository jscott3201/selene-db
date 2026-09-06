---
plan_id: F01-PR03
milestone: F01
initial_status: proposed
---

# F01-PR03 — Store mixed edges and complete their logical change records

**Milestone:** [F01: Finish graph identity and mixed topology](Milestone-F01.md)  
**Dependencies:** [F01-PR02](Milestone-F01-PR-02.md)  
**Carries forward:** M04-PR03, M04-PR05; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-api-design`; `rust-storage-durability`; `rust-test-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Represent directed and undirected edges as first-class state with one edge identity, and carry that representation through logical mutations and snapshot inputs.

## Start from what exists

Stable facade and lower graph identity already exist. Explicit mixed-edge storage and its logical persistence model remain planned. This combines the original storage and change-event work because persistence needs the complete semantic record, not an intermediate directed-only encoding.

**Observed live entry points:** `crates/selene-graph/src/store.rs`, `crates/selene-graph/src/graph.rs`, `crates/selene-core/src`

**Search hints, not verified current filenames:** `crates/selene-graph/src/mutator.rs`, `crates/selene-graph/src/type_validator.rs`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Find the existing edge record, endpoint access, mutation event and graph-type validation owners. Add directionality in the lowest shared semantic layer that avoids crate dependency cycles.
2. Use one stable EdgeId per undirected edge. Canonical endpoint storage is permissible, but stored canonical order must not be confused with source/destination or path traversal orientation.
3. Update creation, property/label mutation, deletion, adjacency maintenance, compaction and copy-on-write paths. Keep parallel edges and self-loops distinct by identity.
4. Extend logical change records and snapshot construction inputs with directionality and stable endpoints. Define versioned logical records for F02; do not finalize a byte format here.
5. Exercise closed graph-type endpoint compatibility and mutation rollback. Keep existing directed graphs behaviorally unchanged.

## Acceptance and concrete regression cases

- [ ] A→B, B→A, an undirected A—B, two parallel A—B edges and both kinds of self-loop remain distinguishable.
- [ ] Removing one parallel edge never removes its sibling; deleting a node removes the correct incident edges exactly once.
- [ ] Undirected endpoint canonicalization is stable under reversed construction without manufacturing two directed edges.
- [ ] An edge with an absent endpoint or a mismatched closed endpoint type fails without publishing partial adjacency.
- [ ] Compaction, logical snapshot reconstruction and change-event application preserve edge identity, properties, directionality and incidence.
- [ ] Aborting a mutation leaves neither a directionality change nor an orphaned index/adjacency entry.
- [ ] Counter exhaustion fails explicitly rather than wrapping/reusing a published identity or leaving a generation transition indistinguishable.

## Validation and performance

Use graph mutation/type-validation/property tests and directed-graph regression fixtures. Add reconstruction tests that inspect semantic records independently of a serializer round trip. Run affected algorithms and GQL tests even before orientation support is completed.

Measure directed-only adjacency and mixed-edge enumeration overhead, memory per edge and mutation cost. Do not duplicate all undirected edges to win a narrow traversal benchmark.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No path automata, persistence byte-layout commitment, weighted path language or distributed graph scope.

## Bridge/deletion boundary

No directed-only logical change bridge survives this PR. F01-PR04 owns query orientation; F02-PR03 owns byte encoding.

## Standards and reviewer focus

§4.3.5 graph model; §4.13.2 edge types; §4.11.2 paths.

**Independent review question:** Is directionality intrinsic to the edge while orientation belongs to its use in a path?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
