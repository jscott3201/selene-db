---
plan_id: F03-PR03
milestone: F03
initial_status: proposed
---

# F03-PR03 — Make effects and logical binding-table operations executable

**Milestone:** [F03: Complete the semantic compiler](Milestone-F03.md)
**Dependencies:** [F03-PR02](Milestone-F03-PR-02.md)
**Carries forward:** M05-PR04, M05-PR05; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-api-design`; `rust-test-design`; `module-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Lower resolved semantics into logical operators with explicit schemas, dependencies and effects, including a usable query and mutation path for subsequent batch work.

## Start from what exists

The transaction authority and semantic type boundary exist. This PR must annotate effects before physical planning while reusing the existing publication funnel; a second planner-specific transaction manager would recreate the split that M03 already removed.

**Observed live entry points:** `crates/selene-gql/src/plan`, `crates/selene-gql/src/runtime`, `crates/selene-db/src/transaction.rs`, `crates/selene-db/src/session.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Define logical operators for binding-table input/output and graph access using semantic descriptors, not source parser nodes or physical row coordinates. Start with scan/filter/project/page and one ordinary mutation path.
2. Classify query, data-modifying, catalog-modifying and session/transaction operations. Resolve named-procedure effects from registration metadata, never by whether its current implementation happens to write.
3. Track conservative write sets and invalidation dependencies. Read-only transactions reject prohibited writes before publication, including indirect writes through procedure calls.
4. Preserve logical ordering, multiplicity, variable scope and type metadata. Mutation operators describe intent; execution still stages changes through the existing detached transaction state.
5. Complete operator contracts needed by physical batches, path semantic nodes and native adapters. F03-PR04 owns full family coverage and removal of the old mixed plan.

## Acceptance and concrete regression cases

- [ ] A read-only transaction rejects a direct mutation and a procedure with data/catalog effects without publishing anything.
- [ ] A query procedure cannot acquire write authority through an unannotated nested call.
- [ ] Logical filter/project/page results retain the same schema and duplicates as the existing reference cases.
- [ ] Multi-statement writes either stage successfully in the active transaction or trigger its documented rollback behavior.
- [ ] Catalog/data statement mixing follows the selected GP18 policy; an unsupported mix is not silently split into separate commits.
- [ ] EXPLAIN/debug output contains stable semantic descriptors and useful source origins, not runtime addresses.

## Validation and performance

Run logical-plan snapshots, effect/transaction negative tests and the existing facade query/mutation corpus through the adapter. Verify an indirect-write fixture so a parser-only effect classifier cannot pass.

Measure lowering and cache dependency construction. Keep write-set computation conservative and cheap before pursuing precision that is not required for single-writer execution.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No physical execution policy in logical nodes, effect inference from operation names alone, independent mutation publication or claim that syntactic purity proves determinism.

## Bridge/deletion boundary

The semantic→current executor adapter remains singular and temporary, owned by F03-PR04. F04 owns physical execution.

## Standards and reviewer focus

§4.8 contexts; §4.10 effect classes; §§12–15 statements/procedures; §4.6 and selected GP18 restrictions.

**Independent review question:** Can a side effect bypass analysis or the one transaction authority through a procedure or catalog operation?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
