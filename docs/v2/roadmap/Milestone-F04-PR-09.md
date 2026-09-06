---
plan_id: F04-PR09
milestone: F04
initial_status: proposed
---

# F04-PR09 — Make batch execution the only production executor

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)  
**Dependencies:** [F04-PR03](Milestone-F04-PR-03.md), [F04-PR04](Milestone-F04-PR-04.md), [F04-PR05](Milestone-F04-PR-05.md), [F04-PR07](Milestone-F04-PR-07.md), [F04-PR08](Milestone-F04-PR-08.md), [F05-PR04](Milestone-F05-PR-04.md), [F03-PR04](Milestone-F03-PR-04.md)  
**Carries forward:** M06-PR07; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-review`; `rust-test-design`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Delete the transitional row executor and logical-to-row adapters after every supported family, including paths and native calls, runs through the batch engine.

## Start from what exists

This cutover deliberately occurs after path integration. Deleting the old executor before native/path coverage would either strand supported behavior or create a hidden production fallback. The independent small reference models remain; the legacy production runtime does not.

**Observed live entry points:** `crates/selene-gql/src/runtime`, `crates/selene-gql/src/plan`, `crates/selene-db/src/session.rs`, `crates/selene-db/src/outcome.rs`, `crates/selene-testing/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Inventory all production dispatch branches by statement family, value kind and feature. Prove that supported paths reach physical batch execution and unsupported features fail explicitly.
2. Run the transition differential suite before deletion, resolving disagreements against the standard/profile and independent fixtures rather than automatically treating the old engine as correct.
3. Remove old evaluator implementations, compatibility adapters, fallback flags and unreachable mixed-plan types. Keep only focused test reference models that do not import the deleted production implementation.
4. Recheck diagnostics, resource limits, cancellation, typed results, cache invalidation and transaction publication through the single route.
5. Update architecture docs and benchmarks to describe what now exists. Run downstream facade fixtures from durable, algorithm and retrieval lanes together.

## Acceptance and concrete regression cases

- [ ] Every supported family in the generated implementation inventory has a batch entry-path test.
- [ ] No runtime option, unsupported-value branch or small-query shortcut invokes the old production executor.
- [ ] Null/empty/omitted results, duplicates, preferred columns and required ordering survive the cutover.
- [ ] Failed writes and indeterminate durable commits retain the same state/diagnostic contract as before.
- [ ] Independent path/type/relational reference fixtures continue to run after old runtime code is physically deleted.
- [ ] An external-style consumer uses one facade with named graph, retrieval, transaction and reopen behavior.

## Validation and performance

Run the full normal PR workspace gate, selected-profile semantic suite, facade integration and separate doctests. A green build with an empty filtered differential test selection is a failure of validation, not successful cutover.

Compare the agreed small-query, scan, join, aggregate, path and retrieval guard workloads. Report regressions with absolute and relative cost and memory; do not retain a second executor as an unexplained performance escape hatch.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No retained production oracle, broad feature removal to make deletion easy, ignored negative tests or new compatibility mode.

## Bridge/deletion boundary

All old row-executor and semantic-to-row execution bridges are deleted. Permanent independent reference models are test-only and intentionally separate.

## Standards and reviewer focus

§4.3.6 binding tables; §§14–16 execution semantics; §23 outcomes; selected profile coverage.

**Independent review question:** Is there exactly one production semantic/execution route, with independent tests still capable of disagreeing with it?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
