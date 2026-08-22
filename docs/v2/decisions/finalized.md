# Finalized Selene DB 2.0 decisions

These decisions are locked for the initial implementation program. An agent
must not reinterpret them. A changed fact triggers REPLAN with evidence,
alternatives, consequences, and the smallest affected plan delta.

<a id="d-001"></a>
- **D-001 · 1.x support:** 1.x is end-of-life immediately. No fixes, security patches, compatibility maintenance, future 1.x release, or persisted migration path.
<a id="d-002"></a>
- **D-002 · Archive boundary:** Archive `b8782bec34ff0b815b62711ac7e33cac09d8ea71` at `archive/1.x-final` and `archive-v1-eol-2026-08-21`. The non-semver archive tag must not trigger publication.
<a id="d-003"></a>
- **D-003 · Product boundary:** Selene DB 2.0 remains an embedded Rust database library. A server, wire protocol, bundled auth service, distributed execution, and loadable extension ABI are out of scope.
<a id="d-004"></a>
- **D-004 · Stable API:** `selene-db` is the stability-promised facade. Lower crates are internal/advanced unless explicitly re-exported and documented.
<a id="d-005"></a>
- **D-005 · Catalog:** One synthetic catalog root directory contains multiple schemas and no child directories. Catalog owns graphs, graph types, procedures, constraints, and indexes.
<a id="d-006"></a>
- **D-006 · Conformance claim:** Claims are generated and evidence-gated. The release may use “ISO/IEC 39075:2024-aligned” with exact gaps, but may not claim minimum/selected-profile conformance until the complete gate is green.
<a id="d-007"></a>
- **D-007 · Concurrency:** Retain serializable single-writer publication with immutable reader snapshots. No MVCC or weaker isolation in the initial 2.0 program.
<a id="d-008"></a>
- **D-008 · Identity:** Node/edge/catalog IDs are stable semantic identity. Rows are private snapshot-scoped physical positions. Public candidate sets are graph-, generation-, and kind-bound.
<a id="d-009"></a>
- **D-009 · Edges:** The physical graph is a mixed multigraph with explicit directed and undirected edge directionality and one identity per edge.
<a id="d-010"></a>
- **D-010 · Contexts:** Session, request, execution, outcome, and transaction are explicit separate objects. Sessions own the database, not one borrowed graph.
<a id="d-011"></a>
- **D-011 · Compiler:** The compiler pipeline is source syntax AST → immutable annotated semantic tree → logical IR → physical plan → runtime operators.
<a id="d-012"></a>
- **D-012 · Execution:** The production executor is pull-based and batch-oriented. Row execution may exist only as a test oracle during transition and is deleted before M06 exit.
<a id="d-013"></a>
- **D-013 · Path engine:** Path expressions lower to automata/product-graph execution with exact mode/match/selective semantics and a permanent small-graph reference oracle.
<a id="d-014"></a>
- **D-014 · Constraints/indexes:** Constraints and indexes are catalog descriptors. Unique/key constraints require complete backing indexes. Single-property uniqueness is arity one.
<a id="d-015"></a>
- **D-015 · Expression indexes:** Index expressions are analyzed, deterministic, pure scalar expressions over one element. JSON scalar paths reuse typed scalar indexes; containment indexes are deferred.
<a id="d-016"></a>
- **D-016 · Filesystem authority:** One anchored `StoreDirectory` capability owns all artifact operations. Path wrappers anchor once and never re-resolve during the operation lifetime.
<a id="d-017"></a>
- **D-017 · Durability authority:** One commit-critical WAL is authoritative. Derived vector/text/JSON/algorithm structures are rebuildable observers unless a future ADR explicitly changes this.
<a id="d-018"></a>
- **D-018 · Commit protocol:** Commit phases are validate → encode/append → flush → publish → acknowledge, with explicit written/flushed/published/acknowledged state and precise canceled/committed/indeterminate outcomes.
<a id="d-019"></a>
- **D-019 · Persisted format:** 2.0 writes and reads only format 2. A bounded header probe may identify 1.x to return an unsupported-format error; no 1.x decoder or migration is shipped.
<a id="d-020"></a>
- **D-020 · Native features:** Algorithms, vectors, BM25/text, JSON, and native procedures return through the facade/catalog/batch system and remain namespaced extensions where not ISO features.
<a id="d-021"></a>
- **D-021 · PR sizing:** Default PR cap is one invariant, at most 25 production files and roughly 1,500 net non-generated lines. Exceeding the cap requires stop/replan or an explicit reviewed exception.
<a id="d-022"></a>
- **D-022 · Review control:** OpenCode opens a non-draft PR and stops. The assistant returns PASS, FIX, or REPLAN. Agents never self-merge 2.0 work.

## Consequence order

1. Correctness, transaction atomicity, and non-destructive recovery.
2. Truthful conformance evidence and structured diagnostics.
3. Stable semantic identity and catalog ownership.
4. Public facade coherence and removal of legacy contracts.
5. Measured embedded latency, throughput, memory, and operational simplicity.
6. Implementation convenience.

## REPLAN boundaries

REPLAN is required before restoring 1.x support, changing the database/catalog
ownership root, adding nested directories, MVCC, multiple commit-critical
providers, a server, plugins, or distributed execution, weakening the evidence
gate, exposing physical rows, retaining a deleted bridge, or permitting an
agent to merge its own work.

Physical choices remain evidence-driven only in their owning work items:
collection backends in M08-PR06, joins in M06-PR04, active-row representation
from M06-PR02, path representation in M07-PR05, WAL codec details in M09-PR04,
and the release claim in M10-PR05. The owner must choose, test, benchmark, and
record the result or retain the simpler option.
