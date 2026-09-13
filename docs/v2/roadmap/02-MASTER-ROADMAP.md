# Master roadmap — finish 2.0 through useful integration points

## Delivery shape

Six finish milestones group **34 implementation PRs**, plus PLAN-01. The live queue has 44 not-fully-merged legacy items; the new queue reduces repeated handoffs while explicitly retaining every remaining behavior. Fewer PRs are not fewer tests or a measured calendar estimate.

The numbering groups related work. **Dependencies on PRs—not milestone numbers—control execution.** Persistence, compiler, constraints and native retrieval should not wait behind unrelated complete milestones.

| Milestone | PRs | What it delivers | Starts after |
|---|---:|---|---|
| [F01](Milestone-F01.md) | 4 | Candidate closure and mixed-edge semantics | PLAN-01 |
| [F02](Milestone-F02.md) | 8 | Anchored format-2 durable embedding and recovery | PLAN-01 for anchoring/declarations; real type/edge prerequisites for codec |
| [F03](Milestone-F03.md) | 4 | One semantic/type/effect/logical compiler | PLAN-01; candidate closure for reference cutover |
| [F04](Milestone-F04.md) | 9 | Batch engine and native algorithms/vector/text/JSON | Logical/type contract and candidate-safe graph |
| [F05](Milestone-F05.md) | 7 | Exact paths, composite/expression indexes, measured read performance | Individual substrate prerequisites; several independent lanes |
| [F06](Milestone-F06.md) | 2 | Public/standards closure and actual release qualification | Completed agreed behavior across lanes |

## Critical interface chain

```text
PLAN-01
  ├─ F01-PR01 → F01-PR02 → mixed-edge storage → orientation
  ├─ F02-PR01 store capability/control
  ├─ F02-PR02 durable declarations
  └─ F03-PR01 semantic entry → F03-PR02 types/references → effects/logical IR

mixed-edge records + durable declarations + types/references + anchored store
  → logical WAL → durable publication → checkpoint/open → lifecycle → recovery
  → F02-PR08 durable preview

logical contracts + candidate-safe mixed graph
  → physical batches + primitives
     ├─ joins/sets and grouping/sort
     ├─ typed native call → algorithms + vector + text/JSON
     └─ path IR → exact modes → selective paths → batch integration
  → one production batch executor

catalog + shared semantic/value contracts
  → complete composite/key indexes → scalar expression/JSON-path indexes

all agreed behavior → truthful claim/API closure → native/package RC → release
```

The diagram summarizes interfaces; [plan.json](plan.json) supplies the precise dependency lists. In particular, path integration precedes deletion of the old row executor. Path IR precedes compiler family cutover, but does not depend on that cutover. These two choices remove potential dependency cycles.

## Useful gates before GA

**In-memory integration.** F01-PR02 supplies a facade-only consumer smoke while preserving the existing session/transaction foundation. Downstream teams can already inventory actual required APIs and begin adapters; they need not wait for every language optimization.

**Durable integration preview.** F02-PR08 establishes the joined format-2 create/open/commit/recover contract. Its dependencies include the durable value/reference and mixed-edge model, but not completed batch execution or advanced path selection. The existing query host may still execute supported queries. This is deliberate integration readiness, not a GA/format-freeze or formal ISO claim.

**Durable native consumer preview.** Join F02-PR08 with F04-PR07/08 and F05-PR05. This covers typed native calls/algorithms, vector/text/JSON retrieval and constrained metadata. Run an actual combined consumer scenario: graph-scoped metadata, parameterized writes, a constraint failure, retrieval, commit/reopen and deletion/invalidation. Do not assume a downstream project is unblocked merely because a crate exists. No other downstream repository was changed or validated in this review.

**GA.** F06-PR02 remains gated on the agreed full finish program, including mixed edges, single-path compiler/batch execution, exact selected paths, indexes, native functions and recovery evidence. ISO-aligned wording with disclosed gaps is governed by the retained claim policy; it does not let the implementer silently drop agreed behavior.

Preview artifact creation is useful, but publishing a version, tag or release still requires the applicable owner authorization. Source/package availability and semantic readiness are separate facts.

## Parallel lanes that actually help

The graph frontier owns candidate lifecycle and raw-row deletion until F01-PR02. A persistence owner can work on capability/control while a compiler owner starts source/semantic separation. Catalog declaration changes need one integration owner because both lanes consume them.

After F03-PR03 and primitive batch contracts, path work, join/group operators and native adapters can proceed separately. Keep shared Value/type/registry files under one assigned owner; a different directory does not prove independence. F05-PR05 constraints and F05-PR07 profiling do not wait for all path work. Serialize performance measurements even when implementation is parallel.

With only one executor, prioritize the candidate frontier, then the shortest ready chain to the durable/native consumer gate. With multiple executors, parallelize independent ownership, not competing implementations of the same state transition. The orchestration model resolves integration order and provides each Luna agent only its required context.

## Why particular items were combined or separated

Mixed-edge storage and logical change records are one model. Physical operator contracts and typed batches need one working scan to prove their shape. Procedure catalog/calls and algorithm adaptation share one boundary. Composite declaration plus incremental enforcement must agree on complete activation. JSON scalar paths should reuse deterministic expression indexes. Release claims and public documentation should close together.

Conversely, joins/set multiplicity and aggregate/group/sort semantics deserve separate review. Recovery’s first working reopen tracer and its full corruption campaign remain separate, as do graph-internal and downstream candidate deletion. Combining them would hide distinct failure surfaces rather than save useful work.

## Complete pending queue

| PR | Outcome | Direct dependencies |
|---|---|---|
| [PLAN-01](PLAN-01.md) | Adopt plan/policy and reconcile old program | None |
| [F01-PR01](Milestone-F01-PR-01.md) | Finish the graph-internal candidate migration | PLAN-01 |
| [F01-PR02](Milestone-F01-PR-02.md) | Remove downstream public-row APIs and close candidate safety | F01-PR01 |
| [F01-PR03](Milestone-F01-PR-03.md) | Store mixed edges and complete their logical change records | F01-PR02 |
| [F01-PR04](Milestone-F01-PR-04.md) | Make traversal and GQL predicates obey mixed-edge semantics | F01-PR03 |
| [F02-PR01](Milestone-F02-PR-01.md) | Anchor store operations and establish format-2 store control | PLAN-01 |
| [F02-PR02](Milestone-F02-PR-02.md) | Unify catalog metadata for constraints, indexes and native registrations | PLAN-01 |
| [F02-PR03](Milestone-F02-PR-03.md) | Encode complete logical transactions in the format-2 WAL | F02-PR01, F02-PR02, F01-PR03, F03-PR02 |
| [F02-PR04](Milestone-F02-PR-04.md) | Connect durable commit to the existing publication authority | F02-PR03 |
| [F02-PR05](Milestone-F02-PR-05.md) | Checkpoint a coherent database and reopen the first durable slice | F02-PR04 |
| [F02-PR06](Milestone-F02-PR-06.md) | Make checkpoint publication, rotation and retention one safe lifecycle | F02-PR05 |
| [F02-PR07](Milestone-F02-PR-07.md) | Close recovery classification and failure evidence | F02-PR06 |
| [F02-PR08](Milestone-F02-PR-08.md) | Cut over exclusively to format 2 and expose a durable integration preview | F02-PR07 |
| [F03-PR01](Milestone-F03-PR-01.md) | Introduce immutable semantic analysis through a working query slice | PLAN-01 |
| [F03-PR02](Milestone-F03-PR-02.md) | Unify structural types, values and reference boundaries | F03-PR01, F01-PR02 |
| [F03-PR03](Milestone-F03-PR-03.md) | Make effects and logical binding-table operations executable | F03-PR02 |
| [F03-PR04](Milestone-F03-PR-04.md) | Complete logical lowering and remove mixed syntax/execution planning | F03-PR03, F05-PR01 |
| [F04-PR01](Milestone-F04-PR-01.md) | Build the physical batch substrate with one working scan | F03-PR03, F01-PR02 |
| [F04-PR02](Milestone-F04-PR-02.md) | Execute primitive query operators in batches | F04-PR01, F01-PR04 |
| [F04-PR03](Milestone-F04-PR-03.md) | Implement joins and set operations without losing multiplicity | F04-PR02 |
| [F04-PR04](Milestone-F04-PR-04.md) | Implement aggregation, grouping and bounded sorting | F04-PR02 |
| [F04-PR05](Milestone-F04-PR-05.md) | Route batch mutations and control operations through one transaction | F04-PR02, F03-PR04 |
| [F04-PR06](Milestone-F04-PR-06.md) | Integrate procedure registration, batch calls and graph algorithms | F04-PR02, F03-PR03, F02-PR02 |
| [F04-PR07](Milestone-F04-PR-07.md) | Restore vector retrieval through the stable native boundary | F04-PR06 |
| [F04-PR08](Milestone-F04-PR-08.md) | Restore text, JSON and maintained providers through the same boundary | F04-PR06 |
| [F04-PR09](Milestone-F04-PR-09.md) | Make batch execution the only production executor | F04-PR03, F04-PR04, F04-PR05, F04-PR07, F04-PR08, F05-PR04, F03-PR04 |
| [F05-PR01](Milestone-F05-PR-01.md) | Lower path semantics into one automata contract | F03-PR03, F01-PR04 |
| [F05-PR02](Milestone-F05-PR-02.md) | Execute bounded product-graph paths and exact mode restrictions | F05-PR01 |
| [F05-PR03](Milestone-F05-PR-03.md) | Implement selective paths and materialize correct path values | F05-PR02 |
| [F05-PR04](Milestone-F05-PR-04.md) | Integrate paths with logical planning and batches, then delete legacy paths | F05-PR03, F04-PR02, F03-PR04 |
| [F05-PR05](Milestone-F05-PR-05.md) | Enforce composite uniqueness and keys incrementally | F02-PR02, F03-PR03, F01-PR02 |
| [F05-PR06](Milestone-F05-PR-06.md) | Reuse scalar indexes for deterministic expressions and JSON paths | F05-PR05, F04-PR02 |
| [F05-PR07](Milestone-F05-PR-07.md) | Resolve measured read-path regressions with balanced evidence | F01-PR02 |
| [F06-PR01](Milestone-F06-PR-01.md) | Close release behavior, public API and truthful GQL claims | F02-PR08, F04-PR09, F05-PR06, F05-PR07 |
| [F06-PR02](Milestone-F06-PR-02.md) | Qualify release artifacts and complete the authorized 2.0 release | F06-PR01 |

## Rescope only on evidence

A required neighboring caller, renamed module or larger mechanical migration is ordinary implementation discovery. Keep it with the owning invariant and explain it. Stop for a consequential scope/public/durable-semantics change, a genuinely unsupported target platform, or a proof that the planned unit cannot be reviewed coherently. Reuse the real failing fixture and propose the smallest change; do not launch another broad architecture pass by default.

No calendar date or throughput multiplier is promised. After the first candidate and durable commits, observed cycle time and integrated consumer evidence can support a better estimate than unweighted PR counts.
