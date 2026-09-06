---
plan_id: F05-PR04
milestone: F05
initial_status: proposed
---

# F05-PR04 — Integrate paths with logical planning and batches, then delete legacy paths

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)
**Dependencies:** [F05-PR03](Milestone-F05-PR-03.md), [F04-PR02](Milestone-F04-PR-02.md), [F03-PR04](Milestone-F03-PR-04.md)
**Carries forward:** M07-PR06; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-test-design`; `rust-review`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Connect the new path engine to physical batch execution and remove the old path evaluator without changing query binding/multiplicity semantics.

## Start from what exists

This must land before F04-PR09. Path lowering and semantic compilation are already complete; integration transports those decisions rather than reparsing source or guessing binding degree in the executor.

**Observed live entry points:** `crates/selene-gql/src/plan`, `crates/selene-gql/src/runtime/expand.rs`, `crates/selene-gql/src/runtime/questioned.rs`, `crates/selene-gql/src/runtime`, `crates/selene-testing/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Lower logical path nodes into physical operators with the pinned snapshot, typed bindings, predicate/effect metadata and resource context.
2. Produce binding batches with correct group variables, conditional singletons, path values and input correlation. Keep graph-pattern joins and reduced-match semantics intact.
3. Compare the selected path corpus against the independent oracle and clause-derived edge cases. Resolve differences according to the source standard and selected profile, not legacy behavior by default.
4. Delete legacy path branches, redundant lowering helpers and fallback behavior once all selected families are integrated. Keep the small independent oracle permanently test-only.
5. Run paths together with filter/project/join/native calls and staged mutations where supported; update generated capability evidence and examples in the same change.

## Acceptance and concrete regression cases

- [ ] Questioned paths and {0,1} patterns differ in exposed binding degree even where their traversed paths coincide.
- [ ] Path results spanning several physical batches preserve input correlation and multiplicity.
- [ ] Selective results retain endpoint partition and predicate-order tests through the facade, not only the direct engine.
- [ ] Errors from path resource exhaustion reach the request outcome and do not leak a successful partial table.
- [ ] Mixed-edge orientation, modes, match modes and zero-edge paths match their shared source fixtures.
- [ ] A source/dispatch inspection finds no production legacy path evaluator.

## Validation and performance

Run the complete selected path differential suite, common-query combinations and compiler/facade regression tests. Include at least one witness for every changed feature/implementation-defined rule; profile inventory alone does not establish its conformance.

Measure whole-query path latency and memory with filtering/joining around the path. Do not hide a slow materialization phase by timing traversal only.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No new path syntax to simplify integration, parallel production path engines, loss of group variables or profile claims generated from merely accepting grammar.

## Bridge/deletion boundary

Legacy path lowering and execution are deleted here. The generic old row executor is removed in F04-PR09.

## Standards and reviewer focus

§§14.4, 16.3–16.12, 22.2–22.4; §24 evidence policy.

**Independent review question:** Do the direct engine and facade exercise exactly the same path semantics and failure boundaries?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
