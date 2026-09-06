---
plan_id: F04-PR02
milestone: F04
initial_status: proposed
---

# F04-PR02 — Execute primitive query operators in batches

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)  
**Dependencies:** [F04-PR01](Milestone-F04-PR-01.md), [F01-PR04](Milestone-F01-PR-04.md)  
**Carries forward:** M06-PR03; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-performance`; `rust-test-design`; `rust-api-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Run scan/match seed, one-hop expansion, filter, project and page through typed batch operators while preserving mixed-edge and null semantics.

## Start from what exists

Typed graph producers and orientation rules are already established. This PR replaces row-at-a-time primitive execution, not semantic analysis, and supplies the operator seam needed for paths and native procedures.

**Observed live entry points:** `crates/selene-gql/src/runtime/scan.rs`, `crates/selene-gql/src/runtime/scan_seed.rs`, `crates/selene-gql/src/runtime/expand.rs`, `crates/selene-gql/src/runtime/property_filter_rows.rs`

**Search hints, not verified current filenames:** `crates/selene-gql/src/plan/optimize`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Use typed graph candidates for scan seeds and index access. Bind/revalidate against the pinned snapshot; never reuse row-backed candidates from a cached plan without their identity checks.
2. Evaluate predicates with the shared type/value service, retaining only True where GQL requires it. Unknown must not be converted to True, an empty string or a host-language falsy substitute.
3. Carry one-hop mixed-edge expansion into batches with correct variable bindings, parallel-edge multiplicity and null extension where supported.
4. Implement projection and OFFSET/LIMIT across batch boundaries without resetting counters per batch. Do not push a limit below an operator unless semantics prove it safe.
5. Feed native/path operators through a stable batch interface and preserve structured completion/error outcomes through result conversion.

## Acceptance and concrete regression cases

- [ ] Indexed versus scan execution agrees on values, duplicates, errors and schema for the same query.
- [ ] Null filters, missing property references and wrong-type operands retain the selected profile behavior.
- [ ] OFFSET and LIMIT combinations span empty/intermediate batches correctly, including zero and exhaustion.
- [ ] A small LIMIT after a multiplicity-producing expansion returns the correct rows rather than limiting the seed prematurely.
- [ ] Mixed-edge loops/parallel edges retain F01-PR04 expected bindings.
- [ ] A stale candidate fails predictably rather than being silently rebound from physical rows.

## Validation and performance

Run primitive batch differential fixtures, existing common-query cases and facade tests. Add a matrix over multiple batch sizes so a boundary-dependent bug cannot pass only under the default size.

Measure tiny queries, label scans, selective property filters, high-degree expansions and limit-short-circuit work. Report rows visited and allocations alongside elapsed time where practical.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No new index semantics, unordered-to-ordered promise, approximate filtering, filter error suppression or accidental graph-generation changes during execution.

## Bridge/deletion boundary

Primitive row operators leave production use as these batch families become authoritative. F04-PR09 deletes any remaining transition adapter.

## Standards and reviewer focus

§14 query statements; §16.13 where; §§16.18–16.19 page; §19 predicates.

**Independent review question:** Can a batch boundary, null or pushed-down limit change the observable answer?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
