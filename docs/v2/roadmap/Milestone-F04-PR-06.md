---
plan_id: F04-PR06
milestone: F04
initial_status: proposed
---

# F04-PR06 — Integrate procedure registration, batch calls and graph algorithms

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)  
**Dependencies:** [F04-PR02](Milestone-F04-PR-02.md), [F03-PR03](Milestone-F03-PR-03.md), [F02-PR02](Milestone-F02-PR-02.md)  
**Carries forward:** M06-PR06, M10-PR01, M10-PR04; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-api-design`; `rust-test-design`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Create one catalog-resolved native call path with typed arguments/results/effects and bring graph algorithms through it immediately, rather than waiting until the release milestone.

## Start from what exists

The facade currently owns a BuiltinProcedureRegistry. F02-PR02 separates durable declarations from runtime implementations. This PR connects that registry to catalog identity and the batch calling convention without persisting function pointers.

**Observed live entry points:** `crates/selene-db/src/database.rs`, `crates/selene-gql/src/runtime/builtins`, `crates/selene-gql/src/runtime/native_algorithms`, `crates/selene-algorithms/src/projection.rs`, `crates/selene-catalog/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Implement named registration and resolution by stable declaration plus runtime signature/effect metadata. Registry changes invalidate dependent plans; unavailable implementations fail explicitly.
2. Validate arity, argument types/defaults and declared result schema once at the call boundary. Preserve per-input-row invocation and optional-call behavior for selected GQL forms.
3. Adapt existing native algorithms to catalog-selected graph handles and private stable-ID projections. Convert output into typed binding batches without exposing graph storage rows.
4. Ensure query algorithms cannot acquire mutation authority. Catalog/data procedures participate in the same effect and transaction contracts as ordinary statements.
5. Exercise at least one algorithm from facade query through registration, analysis, physical call, graph projection and typed result. Complete the existing supported algorithm registration inventory, not a new algorithm feature backlog.

## Acceptance and concrete regression cases

- [ ] Wrong arity/type, stale registration, unavailable implementation and illegal effects fail before execution.
- [ ] A query call over multiple input rows has correct invocation count and no cross-row state leakage.
- [ ] Algorithm outputs refer to stable IDs after deletes/compaction, with no raw projection indices returned.
- [ ] Catalog graph selection chooses the intended graph even where two graphs reuse the same property labels.
- [ ] Reopen restores declarations and reattaches available implementations; unsupported declarations are not silently ignored.
- [ ] Procedure errors preserve source/cause diagnostics and do not accidentally commit staged writes.

## Validation and performance

Run builtin/procedure signature/effect cases and algorithm corpus through the facade/batch route. Compare results with direct algorithm reference fixtures while acknowledging shared numerical kernels. Do not require a new CREATE PROCEDURE grammar: the standard leaves external procedure installation to the implementation.

Measure call overhead, projection construction/reuse, per-row versus batch amortization and small-graph latency. Cache only against validated graph and declaration generations.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No plugin ABI, WASM, bundled server, new algorithm inventory, serialized executable code or separate native transaction coordinator.

## Bridge/deletion boundary

Old native/result conversion and parallel registries are removed once all existing registrations route here. Vector and text/JSON specializations land in F04-PR07/08.

## Standards and reviewer focus

§4.10.2 procedure descriptors/effects; §§15.1–15.3 calls; §16.14 yield; implementation-defined external procedure installation.

**Independent review question:** Are declaration, callable implementation and typed effects resolved together without giving native code a shortcut around the facade?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
