# F06 — Qualify and release 2.0

**Work lane:** Integrated release
**State:** Proposed remaining work; completed legacy foundations are not reopened.
**Start:** [F06-PR01](Milestone-F06-PR-01.md) when its actual dependencies are complete.

## Outcome

Close behavior, public API and truthful claims, then qualify the distributable packages with native failure/recovery evidence.

## PRs and useful ordering

Milestone numbers are grouping labels, not global scheduling barriers. The following dependencies are work-item dependencies; do not wait for an entire earlier milestone when only one interface is needed.

| PR | Deliverable | Prerequisites |
|---|---|---|
| [F06-PR01](Milestone-F06-PR-01.md) | Close release behavior, public API and truthful GQL claims | F02-PR08, F04-PR09, F05-PR06, F05-PR07 |
| [F06-PR02](Milestone-F06-PR-02.md) | Qualify release artifacts and complete the authorized 2.0 release | F06-PR01 |

## Parallel work

Documentation and corpus maintenance accompany each PR; F06 assembles evidence instead of discovering all integration gaps at the end.

Use separate worktrees and one integration owner for shared interfaces. Parallel documentation/test-fixture preparation is useful, but a future dependency is not marked complete because its author supplied a draft type definition.

## Exit evidence

Owner-authorized release uses tested packages, accurate support/format statements and evidence-bounded GQL claims.

The owning PRs carry concrete failure cases. A milestone exit is the combined behavior of its merged work, not a second full redesign or an additional round of administrative approval. Re-run the cross-lane cases affected by integration and record genuine remaining gaps.

## Boundaries retained

The [delivery decisions](07-DECISIONS-AND-DEFERRED-WORK.md) retain embedded Rust, serializable single-writer publication, stable IDs, private graph rows, one authoritative WAL and format-2-only persistence. [Standards notes](04-GQL-AND-DURABILITY-NOTES.md) distinguish required semantics from implementation choices.
