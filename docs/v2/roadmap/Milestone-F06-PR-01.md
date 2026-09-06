---
plan_id: F06-PR01
milestone: F06
initial_status: proposed
---

# F06-PR01 — Close release behavior, public API and truthful GQL claims

**Milestone:** [F06: Qualify and release 2.0](Milestone-F06.md)  
**Dependencies:** [F02-PR08](Milestone-F02-PR-08.md), [F04-PR09](Milestone-F04-PR-09.md), [F05-PR06](Milestone-F05-PR-06.md), [F05-PR07](Milestone-F05-PR-07.md)  
**Carries forward:** M10-PR05, M10-PR06; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-review`; `rust-api-design`; `source-verification` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Finish the public embedding contract, examples and selected-profile evidence, producing a release declaration whose wording does not exceed what the implementation proves.

## Start from what exists

The generated profile and executable evidence harness are already merged. Their existence is infrastructure, not a declaration that all mandatory and selected optional semantics pass. Do not rebuild the profile registry or derive a conformance percentage from accepted grammar.

**Observed live entry points:** `crates/selene-profile/src`, `crates/selene-db/src/lib.rs`, `docs/v2`, `README.md`, `crates/selene-testing/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Enumerate the agreed release feature set and its implication closure from the canonical profile. Confirm every selected runtime family has positive/negative semantic evidence and every in-scope implementation-defined choice is documented.
2. Separate working implementation inventory, passing regression tests and a formal conformance claim. Minimum conformance includes all non-optional syntax/semantics plus its required graph/type/Unicode conditions; a chosen subset cannot redefine that minimum.
3. Review facade public types, error/status bundles, handle lifetimes, session Send/not-Sync policy, durability/open modes, native extensions and bounded-resource failures. Remove leftover temporary re-exports and misleading current-state documentation.
4. Exercise facade-only examples for catalog, mixed graph, transaction/reopen, constraints, vector/text/JSON retrieval and a path query. Describe source-data rebuild instead of any 1.x migration procedure.
5. Publish a clear release-readiness note with exact known gaps and platform assumptions. The original policy permits carefully bounded ISO-aligned wording, not an unsupported minimum-conformance claim. Known incorrect agreed behavior is a defect, not a labeling shortcut.

## Acceptance and concrete regression cases

- [ ] Feature implication closure, Unicode/collation/version choices and in-scope implementation-defined values match the runtime and documentation.
- [ ] Negative programs that must fail do fail with the expected diagnostic category and no unintended effects.
- [ ] No standard claim is inferred solely from the count of registered features or grammar productions.
- [ ] All public examples compile/run against the actual facade and contain no legacy type names or lower construction shortcuts.
- [ ] Documentation describes stable IDs versus process-local handles, indeterminate retry hazards, unsupported format 1 and native extension status accurately.
- [ ] Every agreed functional slice is complete; any proposed release-scope change is visible and requires Justin’s decision rather than being hidden in generated claims.

## Validation and performance

Run the selected-profile evidence harness, full supported semantic corpus, facade examples, separate doctests, API/doc checks and normal PR validation. Confirm that evidence files correspond to actually executed tests rather than hand-authored pass records.

Use existing balanced guards as a release comparison, not a new broad optimization effort. Fix demonstrated release regressions within their owning subsystem; do not reopen architecture solely for aesthetic consistency.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No certification language, custom definition of ISO minimum, concealed mandatory gaps, feature-count progress metric, new optional-feature campaign or unapproved release-scope reduction.

## Bridge/deletion boundary

All temporary public/production bridges from this plan must be gone. Remaining independent test oracles and correct scan alternatives are intentional, not bridge debt.

## Standards and reviewer focus

§24.2 minimum conformance; §24.3 feature implications; §24.5 implementation claims; §24.6 flagger; Annex B in-scope choices.

**Independent review question:** Could an adopter infer a stronger API, durability or standards guarantee than the evidence supports?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
