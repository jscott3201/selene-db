---
plan_id: F04-PR03
milestone: F04
initial_status: proposed
---

# F04-PR03 — Implement joins and set operations without losing multiplicity

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)
**Dependencies:** [F04-PR02](Milestone-F04-PR-02.md)
**Carries forward:** M06-PR04; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-test-design`; `rust-memory-layout`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Complete the batch join and set-operation family with exact duplicate, null, correlated-variable and schema behavior.

## Start from what exists

The old M06-PR04 grouped joins, grouping, sorting and set operations into one large item. Splitting joins/sets from aggregation/sort reduces semantic review risk without creating administrative filler.

**Observed live entry points:** `crates/selene-gql/src/plan`, `crates/selene-gql/src/runtime`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Inventory the selected join/composite-query forms and their shared-variable/type constraints. Reuse the semantic equality/distinctness service rather than comparing serialized values or debug strings.
2. Implement the required join variants and correlated input behavior. Preserve many-to-many multiplicity and the distinction between no match and a row containing null.
3. Implement selected set versus multiset operators with correct schema/column alignment and duplicate counts. Internal hash keys must agree with the language equality relation used for that operation.
4. Account for build-side memory and output amplification before allocation. Bounded memory may return a typed resource error; it must not silently truncate the join.
5. Exercise the operations through logical planning and the stable result boundary, not only standalone batch kernels.

## Acceptance and concrete regression cases

- [ ] A 2×3 repeated-key join produces six required bindings rather than deduplicating them.
- [ ] Empty left/right inputs, unmatched outer rows and null join operands follow the actual selected join rules.
- [ ] Correlated bindings do not leak from one input row into the next.
- [ ] Set and multiset versions of equivalent inputs produce deliberately different duplicate counts.
- [ ] Mixed selected numeric keys and record/reference values use compatible equality and hashing.
- [ ] Large fanout exceeds a budget with a failed outcome rather than a successful partial relation.

## Validation and performance

Use small independent nested-loop/multiset reference tests plus facade queries, randomized batch sizes and existing negative type/scope cases. Comparing only against the previous optimizer is insufficient where both engines could share a wrong rewrite.

Measure selective joins, skewed keys, many-to-many fanout and sparse outer matches. Include peak build/probe memory and tiny-input latency; retain a simple nested-loop path when it is the correct low-overhead choice.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No external spill, arbitrary sort order guarantee, string-encoded comparison keys or cardinality-changing optimization without proof.

## Bridge/deletion boundary

Join/set row adapters are removed from production use; F04-PR09 owns final executor bridge deletion.

## Standards and reviewer focus

§4.3.6 binding-table composition; §14.2 composite query expressions; §§22.12–22.15 comparison/grouping semantics.

**Independent review question:** Do counts and errors match an independent relation model under duplicate-heavy inputs?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
