# F03 — Complete the semantic compiler

**Work lane:** Compiler  
**State:** Proposed remaining work; completed legacy foundations are not reopened.  
**Start:** [F03-PR01](Milestone-F03-PR-01.md) when its actual dependencies are complete.

## Outcome

Separate immutable source and semantic representations, then make type, effect, name and logical-plan decisions in one place.

## PRs and useful ordering

Milestone numbers are grouping labels, not global scheduling barriers. The following dependencies are work-item dependencies; do not wait for an entire earlier milestone when only one interface is needed.

| PR | Deliverable | Prerequisites |
|---|---|---|
| [F03-PR01](Milestone-F03-PR-01.md) | Introduce immutable semantic analysis through a working query slice | PLAN-01 |
| [F03-PR02](Milestone-F03-PR-02.md) | Unify structural types, values and reference boundaries | F03-PR01, F01-PR02 |
| [F03-PR03](Milestone-F03-PR-03.md) | Make effects and logical binding-table operations executable | F03-PR02 |
| [F03-PR04](Milestone-F03-PR-04.md) | Complete logical lowering and remove mixed syntax/execution planning | F03-PR03, F05-PR01 |

## Parallel work

Can begin while candidate cleanup runs; merge the type/reference boundary after F01-PR02. Path IR and physical batch substrates branch from F03-PR03.

Use separate worktrees and one integration owner for shared interfaces. Parallel documentation/test-fixture preparation is useful, but a future dependency is not marked complete because its author supplied a draft type definition.

## Exit evidence

One semantic→logical analysis route; no public ambiguous lower Value/GqlType bridge; diagnostics and generation dependencies survive lowering.

The owning PRs carry concrete failure cases. A milestone exit is the combined behavior of its merged work, not a second full redesign or an additional round of administrative approval. Re-run the cross-lane cases affected by integration and record genuine remaining gaps.

## Boundaries retained

The [delivery decisions](07-DECISIONS-AND-DEFERRED-WORK.md) retain embedded Rust, serializable single-writer publication, stable IDs, private graph rows, one authoritative WAL and format-2-only persistence. [Standards notes](04-GQL-AND-DURABILITY-NOTES.md) distinguish required semantics from implementation choices.
