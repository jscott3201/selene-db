# F01 — Finish graph identity and mixed topology

**Work lane:** Graph frontier
**State:** Proposed remaining work; completed legacy foundations are not reopened.
**Start:** [F01-PR01](Milestone-F01-PR-01.md) when its actual dependencies are complete.

## Outcome

Finish the partially landed candidate migration, then make mixed edges and their logical records correct once for all later consumers.

## PRs and useful ordering

Milestone numbers are grouping labels, not global scheduling barriers. The following dependencies are work-item dependencies; do not wait for an entire earlier milestone when only one interface is needed.

| PR | Deliverable | Prerequisites |
|---|---|---|
| [F01-PR01](Milestone-F01-PR-01.md) | Finish the graph-internal candidate migration | PLAN-01 |
| [F01-PR02](Milestone-F01-PR-02.md) | Remove downstream public-row APIs and close candidate safety | F01-PR01 |
| [F01-PR03](Milestone-F01-PR-03.md) | Store mixed edges and complete their logical change records | F01-PR02 |
| [F01-PR04](Milestone-F01-PR-04.md) | Make traversal and GQL predicates obey mixed-edge semantics | F01-PR03 |

## Parallel work

F02 store anchoring/catalog metadata and F03 semantic work can start after PLAN-01. Serialize writers touching graph storage and candidate lifecycle.

Use separate worktrees and one integration owner for shared interfaces. Parallel documentation/test-fixture preparation is useful, but a future dependency is not marked complete because its author supplied a draft type definition.

## Exit evidence

No public graph storage-row boundary; mixed edges preserve one identity, orientation and logical mutation semantics.

The owning PRs carry concrete failure cases. A milestone exit is the combined behavior of its merged work, not a second full redesign or an additional round of administrative approval. Re-run the cross-lane cases affected by integration and record genuine remaining gaps.

## Boundaries retained

The [delivery decisions](07-DECISIONS-AND-DEFERRED-WORK.md) retain embedded Rust, serializable single-writer publication, stable IDs, private graph rows, one authoritative WAL and format-2-only persistence. [Standards notes](04-GQL-AND-DURABILITY-NOTES.md) distinguish required semantics from implementation choices.
