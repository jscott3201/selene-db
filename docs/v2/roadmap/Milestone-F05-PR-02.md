---
plan_id: F05-PR02
milestone: F05
initial_status: proposed
---

# F05-PR02 — Execute bounded product-graph paths and exact mode restrictions

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)  
**Dependencies:** [F05-PR01](Milestone-F05-PR-01.md)  
**Carries forward:** M07-PR02, M07-PR03; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-test-design`; `rust-memory-layout`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Execute bounded path patterns with correct WALK/TRAIL/SIMPLE/ACYCLIC and graph-match restrictions, backed by a separate small-graph reference model.

## Start from what exists

The new path IR exists, but no correct algorithm can be inferred merely from a graph BFS. A state may include graph position, automaton position, bindings and relevant path history; merging states too aggressively loses valid answers.

**Observed live entry points:** `crates/selene-gql/src/runtime`, `crates/selene-graph/src/graph.rs`, `crates/selene-testing/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Implement product-graph transitions over the pinned mixed graph, producing path bindings rather than only endpoint reachability. Maintain enough state for quantifier scopes and observable captures.
2. Implement WALK without extra uniqueness filtering, TRAIL without repeated edge identities, ACYCLIC without repeated nodes and SIMPLE allowing only the permitted first/last repeat. Apply restrictions at their declared scope.
3. Apply DIFFERENT EDGES or REPEATABLE ELEMENTS according to the explicit/selected default graph match mode. Path mode and match mode are separate constraints; do not assume one implies the other.
4. Build a bounded exhaustive oracle for small graphs that enumerates independently of the optimized traversal. Preserve binding multiplicity and temporary-variable reduction semantics, not merely a set of node sequences.
5. Enforce work/memory/cancellation limits with failed outcomes. A resource limit must not be returned as a successfully complete partial path set.

## Acceptance and concrete regression cases

- [ ] An undirected A—B edge traversed A,B,A is SIMPLE but not TRAIL; SIMPLE therefore cannot be implemented as TRAIL plus a node check.
- [ ] Parallel edges and self-loops distinguish identity-based edge repetition from endpoint repetition.
- [ ] Zero-length paths bind one node and zero edges, rather than an empty list of graph elements.
- [ ] Revisiting the same node/automaton state with different legal path history can yield distinct valid bindings.
- [ ] Nested mode scopes and repeated element variables exercise history restoration on backtracking.
- [ ] Explicit match modes and the registry-defined default produce their independently expected results.

## Validation and performance

Compare optimized and exhaustive results on bounded mixed multigraphs with shrinking property tests. Keep the supplemental selector reference in examples separate: it is not a full pattern/binding oracle. Add negative cases for history loss, mode conflation and incorrectly shared visited sets.

Measure visited product states, allocation and result cardinality across sparse graphs, cycles, fanout and parallel edges. Report output-sensitive work separately from overhead; do not benchmark only endpoint reachability.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No unrestricted enumeration of infinite WALK results, global node-only visited set, default-match-mode guess, silent truncation or weighted-path extension.

## Bridge/deletion boundary

The old multi-step path runtime remains outside this new implementation until F05-PR04. The independent small-graph oracle is permanent test infrastructure.

## Standards and reviewer focus

§4.11.7–4.11.9; §§16.4, 16.6–16.7; §§22.2–22.3.

**Independent review question:** Which dimensions of path history affect future legality or observable bindings, and are they retained in the search state?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
