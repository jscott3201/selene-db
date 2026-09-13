---
plan_id: F03-PR04
milestone: F03
initial_status: proposed
---

# F03-PR04 — Complete logical lowering and remove mixed syntax/execution planning

**Milestone:** [F03: Complete the semantic compiler](Milestone-F03.md)
**Dependencies:** [F03-PR03](Milestone-F03-PR-03.md), [F05-PR01](Milestone-F05-PR-01.md)
**Carries forward:** M05-PR05, M05-PR06; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-api-design`; `rust-test-design`; `rust-review` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Make semantic → logical planning the only analysis route for every supported family, with consistent diagnostics and no parser-node execution fallback.

## Start from what exists

F03-PR01/03 deliberately deliver useful vertical slices instead of a single enormous compiler rewrite. F05-PR01 supplies the path IR contract so this cutover does not invent a second path representation. Runtime batch cutover remains F04-PR09.

**Observed live entry points:** `crates/selene-gql/src/plan`, `crates/selene-gql/src/runtime`, `crates/selene-db/src/request.rs`, `crates/selene-db/src/outcome.rs`, `crates/selene-profile/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Enumerate all supported statement/expression families from the current profile and parser corpus. Route each through semantic descriptors and logical lowering, including catalog, procedure, transaction and path nodes.
2. Unify diagnostic construction and source origins across parser, analyzer and lowering while preserving distinctions between syntax/access, type, feature and runtime failures.
3. Remove mixed analyzed-syntax types and direct parser-node consumption from planning. Keep an explicitly narrow logical-to-old-runtime execution adapter only for the batch transition; do not maintain two analyzers.
4. Update cached-plan dependencies for schema/graph-type/profile/procedure changes and verify that an executing request retains its pinned semantic environment or gets the documented typed invalidation outcome.
5. Run selected-profile evidence generation for changed rules and migrate affected public examples. Fix discovered semantic regressions rather than updating golden files to bless them.

## Acceptance and concrete regression cases

- [ ] Every supported family reaches logical planning; unsupported features fail through the same profile authority with useful spans.
- [ ] Existing good and bad queries preserve values, effects, row schemas and primary/additional statuses.
- [ ] Catalog replacement, procedure signature change and profile change invalidate relevant cached analysis.
- [ ] Path/group-variable metadata survives lowering without replacing conditional singletons with lists or losing scope.
- [ ] A code/API inspection finds no execution dependency on mutable source syntax annotations.
- [ ] Repeated analysis under the same environment remains deterministic while runtime data is not incorrectly frozen into the plan.

## Validation and performance

Run complete compiler/facade semantic corpora, negative diagnostics, selected evidence checks and separate public doctests. Run batch smoke tests that depend on these contracts even if the old executor still hosts other families.

Compare parse/analyze/lower/cache-hit time and memory with the recorded baseline. Explain any intentional increased work with the correctness requirement; do not claim faster execution from analyzer-only measurements.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No grammar expansion, public semantic-tree serialization, random cache invalidation bypass or second path lowering algorithm.

## Bridge/deletion boundary

Old syntax/semantic planning bridge is deleted here. A logical-to-row execution adapter may survive only until F04-PR09, with all semantic decisions already fixed in logical IR.

## Standards and reviewer focus

§22.1 annotation; §23 diagnostics; §24 profile/flagger consistency.

**Independent review question:** Is the compiler genuinely single-path, and does every remaining adapter transport decisions rather than independently rederive them?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
