# F05 — Finish paths, indexes and measured performance

**Work lane:** Semantics and accelerators  
**State:** Proposed remaining work; completed legacy foundations are not reopened.  
**Start:** [F05-PR01](Milestone-F05-PR-01.md) when its actual dependencies are complete.

## Outcome

Complete exact path semantics and useful indexing while resolving proven read/write tradeoffs. This is a set of parallel capability lanes, not a sequential dependency on all of F04.

## PRs and useful ordering

Milestone numbers are grouping labels, not global scheduling barriers. The following dependencies are work-item dependencies; do not wait for an entire earlier milestone when only one interface is needed.

| PR | Deliverable | Prerequisites |
|---|---|---|
| [F05-PR01](Milestone-F05-PR-01.md) | Lower path semantics into one automata contract | F03-PR03, F01-PR04 |
| [F05-PR02](Milestone-F05-PR-02.md) | Execute bounded product-graph paths and exact mode restrictions | F05-PR01 |
| [F05-PR03](Milestone-F05-PR-03.md) | Implement selective paths and materialize correct path values | F05-PR02 |
| [F05-PR04](Milestone-F05-PR-04.md) | Integrate paths with logical planning and batches, then delete legacy paths | F05-PR03, F04-PR02, F03-PR04 |
| [F05-PR05](Milestone-F05-PR-05.md) | Enforce composite uniqueness and keys incrementally | F02-PR02, F03-PR03, F01-PR02 |
| [F05-PR06](Milestone-F05-PR-06.md) | Reuse scalar indexes for deterministic expressions and JSON paths | F05-PR05, F04-PR02 |
| [F05-PR07](Milestone-F05-PR-07.md) | Resolve measured read-path regressions with balanced evidence | F01-PR02 |

## Parallel work

Constraints can start after descriptors/types/candidates; the map investigation starts after candidate closure. Path lowering/execution proceeds alongside primitive/native batches.

Use separate worktrees and one integration owner for shared interfaces. Parallel documentation/test-fixture preparation is useful, but a future dependency is not marked complete because its author supplied a draft type definition.

## Exit evidence

Independent path oracles agree; composite/key enforcement is complete and incremental; scalar JSON paths are indexable; balanced performance decisions are recorded.

The owning PRs carry concrete failure cases. A milestone exit is the combined behavior of its merged work, not a second full redesign or an additional round of administrative approval. Re-run the cross-lane cases affected by integration and record genuine remaining gaps.

## Boundaries retained

The [delivery decisions](07-DECISIONS-AND-DEFERRED-WORK.md) retain embedded Rust, serializable single-writer publication, stable IDs, private graph rows, one authoritative WAL and format-2-only persistence. [Standards notes](04-GQL-AND-DURABILITY-NOTES.md) distinguish required semantics from implementation choices.
