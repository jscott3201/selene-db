---
plan_id: F04-PR01
milestone: F04
initial_status: proposed
---

# F04-PR01 — Build the physical batch substrate with one working scan

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)
**Dependencies:** [F03-PR03](Milestone-F03-PR-03.md), [F01-PR02](Milestone-F01-PR-02.md)
**Carries forward:** M06-PR01, M06-PR02; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-api-design`; `rust-memory-layout`; `rust-test-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Introduce physical operator contracts and typed binding batches together with a working scan-to-result tracer, rather than two disconnected infrastructure PRs.

## Start from what exists

The current executor is row-oriented. The logical schema/effect contract and typed graph candidates now exist. Internal batch positions are not graph storage rows and must never become public graph identities.

**Observed live entry points:** `crates/selene-gql/src/plan`, `crates/selene-gql/src/runtime`, `crates/selene-db/src/outcome.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Define pull-based operator state, initialization, next-batch, completion and error behavior. Keep a single request-owned snapshot/resource context so a batch cannot silently switch graph generation between pulls.
2. Represent column types, nulls, active selections and logical row count explicitly. Preserve the zero-column unit binding table: one row of zero fields is not an empty input.
3. Choose a modest configurable batch policy using current workloads; do not lock in a numeric batch size as a release promise. Keep selection vectors and buffers internal and validate their length/index invariants.
4. Lower and execute a real supported scan into the batch representation, then adapt it to the existing stable result API. Internal batches do not require a new public streaming API.
5. Add a row-reference comparison path for transition tests, not a second production semantic implementation. Build reusable memory-budget and cancellation seams for later operators.

## Acceptance and concrete regression cases

- [ ] Empty input, one row, exactly one batch and one row beyond a batch boundary all have correct cardinality.
- [ ] A zero-column unit input drives one projection/mutation; zero rows drive none.
- [ ] Null bitmaps and selection vectors remain aligned after sparse filtering and buffer reuse.
- [ ] Result columns retain declared types and preferred order, including empty results.
- [ ] Cancellation/error releases the pinned snapshot and temporary buffers without exposing partial mutation.
- [ ] No public API exposes batch row numbers as graph storage rows or stable IDs.

## Validation and performance

Run batch representation property tests and one facade scan differential fixture. Add malformed internal-selection tests at safe constructors, then rely on those invariants in hot loops. Keep doctests separate where result conversion changes.

Compare one-row latency, 1/medium/large scans, allocation count and retained capacity. A throughput improvement that severely regresses common tiny queries needs a deliberate design adjustment, not a hidden adaptive fallback to the old executor.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No JIT, disk spill, public column-layout promise, raw graph rows in batches or scaffolding-only PR with no end-to-end execution.

## Bridge/deletion boundary

The old executor remains transition-only until F04-PR09. The new scan must not call the old evaluator row by row and be presented as a completed batch implementation.

## Standards and reviewer focus

§4.3.6 binding tables; §4.8 execution contexts and unit working table.

**Independent review question:** Does the substrate preserve logical table semantics independently of physical batch shape?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
