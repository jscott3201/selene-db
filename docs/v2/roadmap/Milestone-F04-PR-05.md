---
plan_id: F04-PR05
milestone: F04
initial_status: proposed
---

# F04-PR05 — Route batch mutations and control operations through one transaction

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)
**Dependencies:** [F04-PR02](Milestone-F04-PR-02.md), [F03-PR04](Milestone-F03-PR-04.md)
**Carries forward:** M06-PR05; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-storage-durability`; `rust-test-design`; `rust-api-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Execute data/catalog mutations and session/transaction controls from physical plans while preserving the landed facade authority and durable integration.

## Start from what exists

The public session semantics already work, and F02 extends their publication boundary to persistence. Physical operators must call those services; batch processing must not become one transaction per batch.

**Observed live entry points:** `crates/selene-gql/src/runtime`, `crates/selene-db/src/transaction.rs`, `crates/selene-db/src/session.rs`, `crates/selene-db/src/catalog_stage.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Lower supported mutation/control logical nodes into physical operations that borrow request/transaction state without owning a second writer.
2. Stage every batch of a statement into the same atomic unit. Validate graph type, constraints and affected index maintenance at the shared mutation boundary, including indirect procedure mutations.
3. Preserve explicit multi-request read-your-writes and implicit transaction semantics. Read-only access modes and forbidden catalog/data mixing fail through the existing policy.
4. Define failure and cancellation before/during/following commit: a pre-commit error rolls back as required, while an already durable indeterminate commit is not falsely canceled.
5. Preserve omitted results, write summaries, no-data outcomes and diagnostic bundles through batch/control adapters. Session SET/RESET/CLOSE remain session operations rather than graph writes.

## Acceptance and concrete regression cases

- [ ] A multi-batch insert whose final batch violates a constraint publishes none of that atomic mutation.
- [ ] An explicit transaction sees earlier staged writes; a failed executed procedure triggers the standard-defined rollback attempt and clears state according to the session contract.
- [ ] A read-only transaction rejects direct/indirect writes, including catalog changes where prohibited.
- [ ] Commit failure phases return the same live/recovered-state outcomes as F02-PR04.
- [ ] Session close and cancellation release request state and staged resources without leaking a writer reservation.
- [ ] Unit-table versus empty-table input causes one mutation versus no mutation as specified.

## Validation and performance

Run batch mutation/control tests, existing session/transaction suites and durable facade reopen cases when the persistence lane is available. Until then, keep a named integration case for the joined lane; it must pass before F06, not be counted as executed early.

Measure staged batch mutation cost, durable acknowledgment latency and rollback cleanup. Do not compare an in-memory insert with the previous durable insert and call it a speedup.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No per-batch commit, new autocommit policy, direct graph publication, ignored constraint index failure or unconditional retry after unknown commit outcome.

## Bridge/deletion boundary

Old mutation/control execution adapters are deleted from production use. The authoritative transaction implementation is retained, not replaced.

## Standards and reviewer focus

§4.6; §§7–8; §§12–13; §4.8 outcomes.

**Independent review question:** Is atomicity defined by the transaction/statement contract rather than the arbitrary batch size?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
