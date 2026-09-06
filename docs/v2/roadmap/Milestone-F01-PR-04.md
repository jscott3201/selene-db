---
plan_id: F01-PR04
milestone: F01
initial_status: proposed
---

# F01-PR04 — Make traversal and GQL predicates obey mixed-edge semantics

**Milestone:** [F01: Finish graph identity and mixed topology](Milestone-F01.md)
**Dependencies:** [F01-PR03](Milestone-F01-PR-03.md)
**Carries forward:** M04-PR04; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-test-design`; `rust-api-design`; `rust-review` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Make supported edge-pattern orientations, directed/source/destination predicates and graph traversal agree with explicit mixed-edge storage.

## Start from what exists

F01-PR03 supplies semantic edge records. Existing expand, questioned-pattern and algorithm projection code are live callers; an assumption that every edge has a semantic source and destination is now visible and must be removed at those boundaries.

**Observed live entry points:** `crates/selene-gql/src/runtime/edge_access.rs`, `crates/selene-gql/src/runtime/expand.rs`, `crates/selene-gql/src/runtime/questioned.rs`, `crates/selene-graph/src/graph.rs`, `crates/selene-algorithms/src/projection.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Build a small truth table for every supported full/abbreviated edge-pattern orientation against directed forward, directed backward, undirected and loop edges. Read the grammar/profile before declaring a spelling supported.
2. Centralize matching of a pattern orientation to an edge plus traversal endpoints. Keep intrinsic directionality separate from the chosen path step.
3. Implement the selected directed/source/destination predicates with their standard null/type behavior. For undirected edges, follow the actual predicate rule instead of borrowing the canonical storage order.
4. Preserve graph-pattern binding multiplicity, parallel-edge identity and loop behavior. Specify algorithm projection treatment explicitly where an algorithm requires a directed view.
5. Carry these orientation fixtures into both the existing runtime and the later batch/path engine; the shared test data becomes the integration contract.

## Acceptance and concrete regression cases

- [ ] A mixed graph exercises each supported orientation token and its union of orientations; expected edge IDs come from the truth table.
- [ ] Forward and reverse construction of an undirected edge have equivalent matching behavior.
- [ ] A directed self-loop is eligible for the permitted orientations without accidental double production from adjacency iteration.
- [ ] Parallel edges yield separate bindings unless the GQL operation explicitly removes duplicates.
- [ ] Null and wrong-type predicate operands produce the required value or structured error, not Rust truthiness.
- [ ] Single-step existing execution and a direct stable-ID reference enumeration agree.

## Validation and performance

Run parser/profile compatibility checks only where the change touches grammar; run runtime expansion, predicate, questioned-pattern and graph projection tests. Include a facade query regression, not only direct graph traversal tests.

Measure one-hop directed and mixed expansions with low and high degree. Keep orientation filtering close to adjacency access without bypassing typed-candidate checks.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No default-match-mode change, arbitrary undirected source endpoint, path enumeration rewrite or algorithm-specific graph mutation.

## Bridge/deletion boundary

Legacy directed-only expansion helpers are deleted or narrowed to explicitly directed algorithms. F05-PR04 removes legacy multi-step path execution.

## Standards and reviewer focus

§4.11.2; §16.7 edge patterns; §19.8 directed predicate; §19.10 source/destination predicate.

**Independent review question:** Are self-loops and edge orientation unions tested independently from storage endpoint ordering?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
