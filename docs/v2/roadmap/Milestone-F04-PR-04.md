---
plan_id: F04-PR04
milestone: F04
initial_status: proposed
---

# F04-PR04 — Implement aggregation, grouping and bounded sorting

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)
**Dependencies:** [F04-PR02](Milestone-F04-PR-02.md)
**Carries forward:** M06-PR04; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-test-design`; `rust-performance`; `rust-memory-layout` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Complete batch aggregate/group/order behavior with correct empty-input, null, collation and memory-limit semantics.

## Start from what exists

This is the other half of the former M06-PR04. Type/equality services are shared with joins, but grouping and aggregate empty-input behavior require their own focused proof.

**Observed live entry points:** `crates/selene-gql/src/runtime`, `crates/selene-gql/src/plan`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Implement selected aggregate state transitions, finalization and type promotion using analyzed descriptors. Distinguish COUNT(*) from counting a nullable expression and preserve the specified empty-group result.
2. Use language grouping equivalence, not ordinary comparison predicates, for group keys; nulls that are not distinct belong together where required.
3. Implement sort keys, null ordering, selected collation, preferred result columns and page interaction. Ties are not an implicit total ordering unless the query or public API promises one.
4. Introduce bounded memory accounting for hash state, sort buffers and result amplification. A Top-K optimization is acceptable only when ORDER BY/OFFSET/LIMIT semantics and ties permit it.
5. Keep cancellation/error propagation explicit during accumulation and finalization. A result must not look complete if only some groups or ties were processed.

## Acceptance and concrete regression cases

- [ ] Empty ungrouped aggregation, empty grouped input and one all-null group yield their distinct required results.
- [ ] COUNT(*), COUNT(value), DISTINCT aggregates and duplicate groups exercise separate paths.
- [ ] Numeric overflow, selected NaN/signed-zero behavior and incompatible group values return documented results/errors.
- [ ] Null ordering and collation-sensitive string keys match independently derived fixtures.
- [ ] Multiple batch sizes and partitioning the same input differently do not change the required result.
- [ ] Memory exhaustion/cancellation fails without presenting a truncated complete group or order result.

## Validation and performance

Run independent aggregate/group/sort models for small fixtures and facade query cases. Assert schema and diagnostic information even when row count is zero. Keep numeric tolerance explicit only for operations whose contract permits it.

Measure few/many groups, skew, wide keys, sort with/without LIMIT and memory high-water. Separate ordering cost from projection/result serialization cost.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No spill implementation, host-language null ordering by accident, hidden approximate aggregates or global sorting to satisfy an unordered reference test.

## Bridge/deletion boundary

Aggregate/sort row adapters stop serving production; remaining common executor deletion is F04-PR09.

## Standards and reviewer focus

§14.11 return; §16.15 group by; §§16.16–16.19 order/page; §20.9 aggregates; §22.15 grouping.

**Independent review question:** Are empty input and null grouping treated as semantic cases rather than incidental hash-map behavior?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
