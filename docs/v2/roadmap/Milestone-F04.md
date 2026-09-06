# F04 — Deliver batch execution and native retrieval

**Work lane:** Execution and native adapters  
**State:** Proposed remaining work; completed legacy foundations are not reopened.  
**Start:** [F04-PR01](Milestone-F04-PR-01.md) when its actual dependencies are complete.

## Outcome

Make useful primitive batches and native calls available early, then finish all families and delete the old production executor.

## PRs and useful ordering

Milestone numbers are grouping labels, not global scheduling barriers. The following dependencies are work-item dependencies; do not wait for an entire earlier milestone when only one interface is needed.

| PR | Deliverable | Prerequisites |
|---|---|---|
| [F04-PR01](Milestone-F04-PR-01.md) | Build the physical batch substrate with one working scan | F03-PR03, F01-PR02 |
| [F04-PR02](Milestone-F04-PR-02.md) | Execute primitive query operators in batches | F04-PR01, F01-PR04 |
| [F04-PR03](Milestone-F04-PR-03.md) | Implement joins and set operations without losing multiplicity | F04-PR02 |
| [F04-PR04](Milestone-F04-PR-04.md) | Implement aggregation, grouping and bounded sorting | F04-PR02 |
| [F04-PR05](Milestone-F04-PR-05.md) | Route batch mutations and control operations through one transaction | F04-PR02, F03-PR04 |
| [F04-PR06](Milestone-F04-PR-06.md) | Integrate procedure registration, batch calls and graph algorithms | F04-PR02, F03-PR03, F02-PR02 |
| [F04-PR07](Milestone-F04-PR-07.md) | Restore vector retrieval through the stable native boundary | F04-PR06 |
| [F04-PR08](Milestone-F04-PR-08.md) | Restore text, JSON and maintained providers through the same boundary | F04-PR06 |
| [F04-PR09](Milestone-F04-PR-09.md) | Make batch execution the only production executor | F04-PR03, F04-PR04, F04-PR05, F04-PR07, F04-PR08, F05-PR04, F03-PR04 |

## Parallel work

Joins and grouping/sort can have separate owners after the primitive contract. Native vector and text/JSON adapters can proceed independently after the call contract, with one owner for shared registry edits.

Use separate worktrees and one integration owner for shared interfaces. Parallel documentation/test-fixture preparation is useful, but a future dependency is not marked complete because its author supplied a draft type definition.

## Exit evidence

One production batch executor supports all selected families, native algorithms, vector retrieval, BM25/text and JSON; old row runtime is deleted.

The owning PRs carry concrete failure cases. A milestone exit is the combined behavior of its merged work, not a second full redesign or an additional round of administrative approval. Re-run the cross-lane cases affected by integration and record genuine remaining gaps.

## Boundaries retained

The [delivery decisions](07-DECISIONS-AND-DEFERRED-WORK.md) retain embedded Rust, serializable single-writer publication, stable IDs, private graph rows, one authoritative WAL and format-2-only persistence. [Standards notes](04-GQL-AND-DURABILITY-NOTES.md) distinguish required semantics from implementation choices.
