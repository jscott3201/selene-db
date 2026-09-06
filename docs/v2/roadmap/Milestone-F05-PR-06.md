---
plan_id: F05-PR06
milestone: F05
initial_status: proposed
---

# F05-PR06 — Reuse scalar indexes for deterministic expressions and JSON paths

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)
**Dependencies:** [F05-PR05](Milestone-F05-PR-05.md), [F04-PR02](Milestone-F04-PR-02.md)
**Carries forward:** M08-PR04, M08-PR05; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** #1097
**Focused skills:** `rust-test-design`; `rust-storage-durability`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Index analyzed pure scalar expressions—including selected JSON scalar paths—and let the planner use them only when semantic equivalence is established. Close #1097.

## Start from what exists

F02 owns declarations and F05-PR05 provides lifecycle-complete typed indexes. This extends those targets rather than introducing a separate JSON-specific indexing engine. Existing correct scans remain the reference and fallback when no usable index exists.

**Observed live entry points:** `crates/selene-gql/src/plan/optimize/index_catalog.rs`, `crates/selene-gql/src/plan/optimize/live_index_catalog.rs`, `crates/selene-graph/src/json_search.rs`, `crates/selene-db/src/catalog_stage.rs`

**Search hints, not verified current filenames:** `crates/selene-graph/src/index_provider.rs`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Analyze expression targets through the shared semantic/type service. Admit only deterministic, pure, same-element scalar expressions with known dependencies and a stable semantic/profile identity.
2. Reject parameters, time/randomness, external procedures, cross-element traversal, side effects and multi-valued results unless explicitly brought into scope through a new reviewed contract.
3. Define JSON scalar-path extraction precisely: absent versus JSON null versus a typed scalar, wrong container/type, escapes and selected path syntax. Do not use string coercion to make every value indexable.
4. Maintain indexes across create/update/remove/delete, rollback, replay, graph/type changes and rebuild. Publish usable state only when complete for its snapshot.
5. Match query expressions structurally with their type/collation/null/error semantics. If equivalence is not proven, use a correct scan and residual predicate; an index may narrow candidates but must not suppress observable errors or required rows.

## Acceptance and concrete regression cases

- [ ] Indexed and unindexed paths agree for missing/JSON-null/scalar values, numeric forms and collation-sensitive strings.
- [ ] Wrong-type and invalid path inputs retain specified errors rather than becoming silent nonmatches.
- [ ] Parameterized, nondeterministic and multi-valued index targets are rejected explicitly.
- [ ] Changing a source property updates all dependent expression keys and rollback restores the previous index state.
- [ ] Equivalent expressions use the intended index; superficially similar expressions with different type/null/collation semantics do not.
- [ ] Reopen/rebuild and graph replacement invalidate stale expression metadata; selective queries show reduced candidate work without changing answers.

## Validation and performance

Run indexed-versus-scan differential tests with independently authored boundary fixtures, lifecycle mutation sequences and planner selection/explain assertions. Test error equivalence as well as row equality. Reuse the existing scalar target engine instead of adding a second tokenizer/parser.

Measure selective/nonselective JSON-path queries, index maintenance and rebuild memory. Show actual visited candidates/plan choice; an index existence flag is not performance evidence.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No GIN containment index, arbitrary expression language, side-effecting target, stringly typed tuple keys, swallowed evaluation errors or implicit feature expansion.

## Bridge/deletion boundary

Ad-hoc JSON scalar index paths are consolidated into the common expression target. Correct scan execution remains a permanent alternative, not compatibility debt.

## Standards and reviewer focus

§19 predicates; §20 expressions; §22 type/comparison rules. Expression/JSON indexing is a Selene implementation/extension facility.

**Independent review question:** Is the planner’s index equivalence proof as strong as the runtime expression semantics, including errors?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
