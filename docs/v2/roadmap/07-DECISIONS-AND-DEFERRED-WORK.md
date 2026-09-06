# Retained decisions and narrowly revised delivery policy

The uploaded plan contains 22 finalized decisions. Their product/semantic intent remains; only the delivery mechanics in D-021/D-022 are deliberately revised by PLAN-01. This page does not pretend an unverified owner-only archive/tag action has occurred.

| Original decision | Finish-plan treatment |
|---|---|
| D-001 — 1.x EOL | Retain the clean break; no 1.x maintenance/compatibility/migration work in this program |
| D-002 — historical archive | Preserve historical references; any unresolved owner-only archival action remains separate from product execution |
| D-003 — embedded library | No server, wire protocol, bundled auth, distributed execution or loadable extension ABI |
| D-004 — stable facade | selene-db is the supported embedding surface; lower crates remain internal/advanced unless intentionally exposed |
| D-005 — catalog shape | One synthetic root with schemas, no nested directory or multi-catalog expansion; catalog owns declarations |
| D-006 — claims | Evidence-gated claims; precise ISO-aligned wording is distinct from formal minimum/feature conformance |
| D-007 — transactions | Serializable single-writer publication and immutable reader snapshots; no new MVCC/weaker isolation |
| D-008 — identity | Stable semantic IDs; private snapshot-local graph rows and validated typed candidates |
| D-009 — mixed graph | One intrinsic directionality and identity per edge; parallel edges and loops are real cases |
| D-010 — contexts | Separate session/request/execution/outcome/transaction; sessions retain database ownership |
| D-011 — compiler | Source AST → immutable semantic tree → logical IR → physical plan → runtime |
| D-012 — executor | Pull-based batches; delete the old production row executor in F04-PR09 |
| D-013 — paths | Automata/product-graph semantics with a permanent independent bounded reference model |
| D-014 — constraints | Catalog-owned unique/key declarations with complete backing indexes; arity one is not a second engine |
| D-015 — expression indexes | Pure deterministic same-element scalar targets; JSON scalar paths reuse typed indexing |
| D-016 — directory authority | Anchored StoreDirectory for all managed artifact operations |
| D-017 — WAL authority | One commit-critical WAL; accelerators are rebuildable observers, not independent commit voters |
| D-018 — commit phases | Prepare/append/sync/publish/acknowledge; precise canceled/committed/indeterminate outcomes |
| D-019 — format | Read/write format 2 only; bounded old-header rejection, no old decoder or migration |
| D-020 — native capabilities | Algorithms, vector, BM25/text, JSON and procedures use the facade/catalog/batch system with accurate extension labels |
| D-021 — PR sizing | Revise: coherent behavior and evidence define the PR; necessary callers are not blocked by fixed path/file/net-line inventories |
| D-022 — roles/review | Revise: Luna implements; orchestrator integrates and performs authorized Git/PR work; proportionate independent review replaces automatic paired-review ceremony |

PLAN-01 must synchronize the actual repository decision/review/operating documents and validator before the revised mechanics apply. Required checks, current-change review, user authorization, safety gates and the workspace prohibition on unsafe code remain in force.

## Explicitly deferred, not forgotten

Server/wire protocol and bundled authentication; nested catalog directories/multiple catalogs; multiple writers/MVCC/weaker isolation; loadable dynamic/WASM extension ABI; distributed graph execution; JIT; disk spill for joins/groups/sort; GIN-style JSON containment; weighted-path language extensions; GPU production requirements; persistent accelerator internals as authority; and every optional GQL feature remain outside the initial agreed program.

No Python/PyO3 crate appears in the inspected workspace. A downstream binding effort may follow the stable facade, but inventing one as a 2.0 release blocker would broaden this request and delay the database’s consumers.

The program retains the no-migration 1.x boundary. A format/API preview is not a promise to preserve every pre-GA experiment forever; deliberate downstream integration should use controlled fixtures and source data until the supported contract is adopted.

A deferred capability earns new work only from a concrete product need and an appropriately sized implementation proposal. It is not a reason to append another broad architecture program to the release tail.
