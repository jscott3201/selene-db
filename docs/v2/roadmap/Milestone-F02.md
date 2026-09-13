# F02 — Bring durable embedding forward

**Work lane:** Persistence and facade
**State:** Proposed remaining work; completed legacy foundations are not reopened.
**Start:** [F02-PR01](Milestone-F02-PR-01.md) when its actual dependencies are complete.

## Outcome

Extend the existing in-memory transaction authority into one recoverable format-2 database, without waiting for the complete batch/path program.

## PRs and useful ordering

Milestone numbers are grouping labels, not global scheduling barriers. The following dependencies are work-item dependencies; do not wait for an entire earlier milestone when only one interface is needed.

| PR | Deliverable | Prerequisites |
|---|---|---|
| [F02-PR01](Milestone-F02-PR-01.md) | Anchor store operations and establish format-2 store control | PLAN-01 |
| [F02-PR02](Milestone-F02-PR-02.md) | Unify catalog metadata for constraints, indexes and native registrations | PLAN-01 |
| [F02-PR03](Milestone-F02-PR-03.md) | Encode complete logical transactions in the format-2 WAL | F02-PR01, F02-PR02, F01-PR03, F03-PR02 |
| [F02-PR04](Milestone-F02-PR-04.md) | Connect durable commit to the existing publication authority | F02-PR03 |
| [F02-PR05](Milestone-F02-PR-05.md) | Checkpoint a coherent database and reopen the first durable slice | F02-PR04 |
| [F02-PR06](Milestone-F02-PR-06.md) | Make checkpoint publication, rotation and retention one safe lifecycle | F02-PR05 |
| [F02-PR07](Milestone-F02-PR-07.md) | Close recovery classification and failure evidence | F02-PR06 |
| [F02-PR08](Milestone-F02-PR-08.md) | Cut over exclusively to format 2 and expose a durable integration preview | F02-PR07 |

## Parallel work

Start directory and descriptor work immediately. The logical codec waits for mixed-edge records and F03-PR02 value/reference semantics, but not for batch cutover or advanced paths.

Use separate worktrees and one integration owner for shared interfaces. Parallel documentation/test-fixture preparation is useful, but a future dependency is not marked complete because its author supplied a draft type definition.

## Exit evidence

Facade create/open/commit/checkpoint/recover/verify work on format 2 only; canceled, committed and indeterminate outcomes have failure evidence.

The owning PRs carry concrete failure cases. A milestone exit is the combined behavior of its merged work, not a second full redesign or an additional round of administrative approval. Re-run the cross-lane cases affected by integration and record genuine remaining gaps.

## Boundaries retained

The [delivery decisions](07-DECISIONS-AND-DEFERRED-WORK.md) retain embedded Rust, serializable single-writer publication, stable IDs, private graph rows, one authoritative WAL and format-2-only persistence. [Standards notes](04-GQL-AND-DURABILITY-NOTES.md) distinguish required semantics from implementation choices.
