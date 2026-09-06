---
plan_id: F05-PR01
milestone: F05
initial_status: proposed
---

# F05-PR01 — Lower path semantics into one automata contract

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)  
**Dependencies:** [F03-PR03](Milestone-F03-PR-03.md), [F01-PR04](Milestone-F01-PR-04.md)  
**Carries forward:** M07-PR01; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-api-design`; `rust-test-design`; `module-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Define path semantic IR and automata lowering with explicit binding, orientation, quantifier and mode metadata, ready for compiler and runtime consumers.

## Start from what exists

F03 already supplies names, types and logical context, and F01 supplies mixed-edge orientation. This small boundary must land before compiler cutover and path execution; it does not depend on either final cutover, avoiding a circular plan.

**Observed live entry points:** `crates/selene-gql/src/plan`, `crates/selene-gql/src/runtime`, `crates/selene-gql/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Inventory supported path syntax/profile features and carry source origins into path semantic nodes. Keep element identities, temporary variables, conditional singletons and group references explicit.
2. Lower node/edge tests, concatenation, selected alternation and quantified/grouped patterns into a representation whose transitions preserve binding scope and multiplicity.
3. Distinguish the questioned-path primary from {0,1}: they may traverse equivalent lengths but expose variables differently. Do not erase that distinction while normalizing automata.
4. Represent intrinsic edge directionality and accepted traversal orientations separately. Carry local path-mode scope, graph match mode and selective-prefix metadata without selecting an algorithm prematurely.
5. Provide stable debug fixtures and the contract consumed by F03-PR04 and F05-PR02. Do not implement a second parser or a second type resolver inside path lowering.

## Acceptance and concrete regression cases

- [ ] Questioned versus bounded-quantified paths retain conditional-singleton versus group binding metadata.
- [ ] Concatenation and alternation preserve variable scope, repeated declarations and selected set/multiset semantics.
- [ ] Nested quantifiers and local modes do not silently become one global restriction.
- [ ] Mixed-edge orientation tokens lower to the expected acceptance tests.
- [ ] Invalid unbounded patterns are rejected according to finite-result restrictions rather than assigned an arbitrary runtime hop cap.
- [ ] Source spans remain useful after normalization introduces temporary semantic variables.

## Validation and performance

Run path-lowering snapshots, invalid scope/degree cases and parser/profile tests for touched rules. Add independently authored binding-metadata expectations; a snapshot copied from the new lowerer is not its own proof.

Measure automaton state/transition counts and lowering time for representative nested patterns. Put explicit resource limits on pathological source expansion before allocating exponentially sized structures.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No automaton-state-only visited rule, runtime traversal here, grammar expansion, erased group metadata or arbitrary hard hop limit replacing language semantics.

## Bridge/deletion boundary

No new syntax fallback. F05-PR04 deletes the old path planner/runtime after the new execution contract is complete.

## Standards and reviewer focus

§4.11.3–4.11.5; §§16.3–16.12; §§22.2–22.4.

**Independent review question:** Can two syntactically similar patterns with different binding semantics still be distinguished after lowering?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
