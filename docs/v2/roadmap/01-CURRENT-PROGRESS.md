# Current 2.0 progress and the acceleration opportunity

## What the source supports

The live workspace is on `development`, version **2.0.0-alpha.1**, edition 2024, with a declared Rust floor of **1.97.1** and **nine workspace crates**. The tracked plan has **65 work items** across 11 milestones. The supplied archive began with 64; the live M01 registry/harness split added one. There are **21 fully merged plan items**, plus the first two implementation parts of M04-PR02. This is an accounting of plan state, not a percentage estimate of remaining engineering effort. [S01, S05]

| Area | Observed state | Consequence for the finish plan |
|---|---|---|
| Program/baseline (M00) | Four work items merged | Keep the baseline and ordinary repo gates; do not repeat installation |
| Executable profile/evidence (M01) | Six live items merged | Reuse profile authority; evidence inventory is not formal conformance |
| Catalog/facade (M02) | Five items merged | Extend ownership instead of introducing another database wrapper |
| Sessions/requests/transactions (M03) | Five items merged | Extend one existing publication authority with persistence |
| Stable identity (M04-PR01) | Merged | Preserve stable handles and private physical coordinates |
| Typed candidates (M04-PR02) | Parts 1/2 landed; 3A/3B remain | Finish this migration before changing its consumer contracts again |
| Mixed edges/compiler/batches/paths/indexes/format 2 | Remaining roadmap work | Deliver in explicit interacting lanes, not a complete serial chain |

The facade source explicitly describes its current database as in-memory. It has named schemas/graphs/closed graph types, lifetime-free sessions, typed request context, detached transaction state and one outer in-memory publication. Persistent open/recovery remains future work. Existing lower WAL/snapshot code is substantial reusable implementation, but its presence does not make the new facade durable. [S05, S06, S08]

## The immediate planning blocker is concrete

The latest reviewed planning change, **PR #1181**, split candidate Part 3 into **3A (graph-internal bridge removal)** and **3B (downstream/public-row removal)**. Its explanation identified required callers outside the earlier fixed inventory: ten production uses across nine graph files and additional GQL consumers. The new contract still imposes exact inventories, 25-production-file/1,500-net-line limits and a fixed number of implementation parts. [S02, S03]

This shows a real mismatch between semantic ownership and delivery mechanics. It does not prove that code quality reviews are unnecessary. The recommended correction is to retain candidate identity, lifecycle and deletion proofs while allowing necessary callers/tests in the same coherent change. PLAN-01 makes that policy revision explicit before Luna executes the new scope.

## Important implementation boundaries already present

CandidateSet is typed by node versus edge kind, bound to graph identity/generation and to retained physical-layout and workspace-binding tokens. It returns stable IDs publicly. Both graph-internal trusted_rows bridge methods still exist in the reviewed source. Generic stable-ID binding sorts/deduplicates and filters non-live IDs; vector scoring separately decides whether a live node has a usable vector. Do not delete or redefine the existing unbound stable-ID VectorCandidateSet while removing graph storage-row APIs. [S04]

The facade’s Value and GqlType re-exports are explicitly temporary. In particular, bare-ID reference variants in the lower Value are not the same as database-scoped facade handles. F03-PR02 resolves that boundary before F02-PR03 seals durable value encoding. This is why persistence can start early but cannot simply serialize today’s public Value and declare format 2 finished. [S05]

MutationIndeterminate currently documents a complete mutation already visible in memory. Durable ambiguity can arise earlier, after an uncertain write/sync or cleanup result. F02-PR04 must preserve that distinction or deliberately revise the pre-GA API with its documentation/callers; it cannot silently broaden the existing meaning. [S05, S06]

The current WAL writer exposes a committed_offset and flushes with sync_data without a retained flushed-offset watermark. Its existing tail-repair behavior and path-based artifact authority need review in the new store protocol. These are concrete reasons to bring #1088/#1128 and format-2 recovery forward, not reasons to replace all persistence code. [S07, S08]

## What should change in the roadmap

**Finish instead of restart.** Retain 21 completed items and landed candidate parts as history. New finish IDs only own remaining outcomes.

**Shorten integration latency.** Store capability/control and semantic analysis begin while the candidate frontier completes. Logical WAL encoding follows stable mixed-edge/value/declaration contracts, not a fully replaced query engine. Native procedures/retrieval follow primitive batches, not the final release milestone.

**Use vertical evidence.** A first durable checkpoint includes real facade reopen; physical batches include a working scan; catalog registration includes an actual algorithm call. Later hardening closes the larger risk surface without pretending those tracers already prove it.

**Collapse shared work and split independent risks.** Combine edge storage with logical change events; declarations with incremental constraint enforcement; expression indexes with JSON scalar paths; procedure calling with native registration. Split the former joins/grouping/aggregate/sort mega-item into two behaviorally distinct PRs.

**Keep the good validation posture.** Existing PR CI already performs Linux workspace checks/nextest; release/nightly lanes cover heavier work. The biggest immediate gain is eliminating repeated release-style local gates and mechanical planning churn, not removing real tests. [S11]

## Review limits

This was read-only source/roadmap analysis. No Rust workspace build, test, benchmark, fuzz campaign, crash campaign or downstream repository integration was executed. No progress percentage, ship date or measured speedup is asserted. The package’s supplemental Python reference tests and structural checks are reported separately in PACKAGE-CHECK.md and must not be mistaken for Selene runtime evidence.

Sources are listed in [SOURCE-NOTES.md](SOURCE-NOTES.md); live work-item coverage is in [06-OLD-TO-NEW-MAP.md](06-OLD-TO-NEW-MAP.md).
