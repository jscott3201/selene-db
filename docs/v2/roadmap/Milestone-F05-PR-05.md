---
plan_id: F05-PR05
milestone: F05
initial_status: proposed
---

# F05-PR05 — Enforce composite uniqueness and keys incrementally

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)
**Dependencies:** [F02-PR02](Milestone-F02-PR-02.md), [F03-PR03](Milestone-F03-PR-03.md), [F01-PR02](Milestone-F01-PR-02.md)
**Carries forward:** M08-PR02, M08-PR03; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** #1092, #1094
**Focused skills:** `rust-storage-durability`; `rust-test-design`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Activate catalog-owned composite UNIQUE/key constraints only with complete backing indexes, then enforce transaction deltas incrementally. Close #1092 and #1094 together.

## Start from what exists

Composite declaration and incremental enforcement were separate old PRs, risking a period of active full-scan or incomplete semantics. One coherent activation/enforcement slice gives downstream metadata consumers useful constraints earlier and avoids maintaining arity-one as a separate subsystem.

**Observed live entry points:** `crates/selene-db/src/catalog_stage.rs`, `crates/selene-db/src/transaction.rs`

**Search hints, not verified current filenames:** `crates/selene-graph/src/type_validator.rs`, `crates/selene-graph/src/type_validator/unique.rs`, `crates/selene-graph/src/index_provider.rs`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Define ordered component targets and typed composite keys using the shared equality/collation service. Do not encode tuples by concatenating displayed values or delimiters.
2. State null/missing behavior separately for UNIQUE and key constraints as Selene extension/profile policy where applicable. Do not import SQL behavior silently or claim property uniqueness syntax is ISO GQL merely because graph types are standardized.
3. Build/validate a complete index against a pinned graph state before activating the constraint. Under single-writer authority, either hold the appropriate reservation or revalidate the build against current generation before publication.
4. Apply transaction deltas against the final staged state, handling deletes and updates before deciding conflicts. Support key swaps within one atomic transaction where the declared semantics permit them; avoid order-dependent false duplicate errors.
5. Route ordinary writes, native writes, batch writes, schema changes and recovery through the same constraint service. Required backing-index failure blocks activation/open-for-write, not merely query optimization.

## Acceptance and concrete regression cases

- [ ] Arity-one and arity-N constraints use one implementation; typed tuples with delimiters, null/missing and equivalent numeric forms cannot collide or diverge incorrectly.
- [ ] Activation over pre-existing duplicates fails without advertising an active constraint.
- [ ] Two inserts of the same key in one transaction conflict; deletion/reuse and permitted key swaps are judged on final state, not incidental batch order.
- [ ] Node and edge domains, multiple graphs and graph-type replacement cannot contaminate one another’s indexes.
- [ ] Rollback and failed publication leave both data and backing index unchanged; reopen reconstructs the same enforced state.
- [ ] A one-element update in a large graph uses bounded affected-key work rather than rescanning every live element.

## Validation and performance

Run independent final-state constraint models, mutation/index lifecycle tests and facade declaration/write cases. Test both the existing execution host and batch mutation integration as available; full integration is required before GA. Use counters/shape checks to prove incremental work, not timing alone.

Measure activation separately from per-transaction enforcement at multiple graph sizes and key arities. Include mixed create/update/delete workloads and rollback, not just append-only insert.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No active constraint over an incomplete index, derived string key workaround, independent per-property unique implementation, unreviewed null semantics or ANN/text index as a uniqueness proof.

## Bridge/deletion boundary

Legacy full-live-scan commit enforcement and arity-one special paths are deleted once the catalog-backed service is active. A full scan remains valid for initial activation/verification, not each small commit.

## Standards and reviewer focus

§4.13 graph-type obligations; equality/collation/value rules. Named uniqueness/key facilities must be identified accurately in the Selene extension profile.

**Independent review question:** Can final-state correctness and incremental cost both be demonstrated across every mutation and recovery entry point?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
