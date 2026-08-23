# Selene DB 2.0 milestones

<!-- Generated from plan.json; do not edit by hand. -->

The dependency fields in the machine plan are authoritative. Work may overlap only after every listed dependency is satisfied.

| ID | Milestone | Depends on | Work items |
|---|---|---|---|
| M00 | 2.0 Pivot, 1.x End-of-Life, Governance, and Baselines | None | [M00-PR01](work-items-00-04.md#m00-pr01), [M00-PR02](work-items-00-04.md#m00-pr02), [M00-PR03](work-items-00-04.md#m00-pr03), [M00-PR04](work-items-00-04.md#m00-pr04) |
| M01 | Executable GQL Conformance Profile and Evidence System | M00 | [M01-PR01](work-items-00-04.md#m01-pr01), [M01-PR02](work-items-00-04.md#m01-pr02), [M01-PR03](work-items-00-04.md#m01-pr03), [M01-PR04](work-items-00-04.md#m01-pr04), [M01-PR05](work-items-00-04.md#m01-pr05), [M01-PR06](work-items-00-04.md#m01-pr06) |
| M02 | Database Facade and Catalog Ownership | M00, M01-PR01 | [M02-PR01](work-items-00-04.md#m02-pr01), [M02-PR02](work-items-00-04.md#m02-pr02), [M02-PR03](work-items-00-04.md#m02-pr03), [M02-PR04](work-items-00-04.md#m02-pr04), [M02-PR05](work-items-00-04.md#m02-pr05) |
| M03 | Session, Request, Execution, and Transaction Semantics | M02 | [M03-PR01](work-items-00-04.md#m03-pr01), [M03-PR02](work-items-00-04.md#m03-pr02), [M03-PR03](work-items-00-04.md#m03-pr03), [M03-PR04](work-items-00-04.md#m03-pr04), [M03-PR05](work-items-00-04.md#m03-pr05) |
| M04 | Stable Identity, Typed Candidate Sets, and Mixed Edge Directionality | M02, M03-PR01 | [M04-PR01](work-items-00-04.md#m04-pr01), [M04-PR02](work-items-00-04.md#m04-pr02), [M04-PR03](work-items-00-04.md#m04-pr03), [M04-PR04](work-items-00-04.md#m04-pr04), [M04-PR05](work-items-00-04.md#m04-pr05) |
| M05 | Semantic Compiler and Logical GQL IR | M02, M03, M04-PR01 | [M05-PR01](work-items-05-10.md#m05-pr01), [M05-PR02](work-items-05-10.md#m05-pr02), [M05-PR03](work-items-05-10.md#m05-pr03), [M05-PR04](work-items-05-10.md#m05-pr04), [M05-PR05](work-items-05-10.md#m05-pr05), [M05-PR06](work-items-05-10.md#m05-pr06) |
| M06 | Physical Planning and Batch-Oriented Execution | M05, M04-PR02 | [M06-PR01](work-items-05-10.md#m06-pr01), [M06-PR02](work-items-05-10.md#m06-pr02), [M06-PR03](work-items-05-10.md#m06-pr03), [M06-PR04](work-items-05-10.md#m06-pr04), [M06-PR05](work-items-05-10.md#m06-pr05), [M06-PR06](work-items-05-10.md#m06-pr06), [M06-PR07](work-items-05-10.md#m06-pr07) |
| M07 | GQL Path Pattern and Selective Search Engine | M05, M06-PR02, M04 | [M07-PR01](work-items-05-10.md#m07-pr01), [M07-PR02](work-items-05-10.md#m07-pr02), [M07-PR03](work-items-05-10.md#m07-pr03), [M07-PR04](work-items-05-10.md#m07-pr04), [M07-PR05](work-items-05-10.md#m07-pr05), [M07-PR06](work-items-05-10.md#m07-pr06) |
| M08 | Catalog-Owned Constraints, Expression Indexes, JSON Paths, and Storage Performance | M02, M04, M05, M06-PR03 | [M08-PR01](work-items-05-10.md#m08-pr01), [M08-PR02](work-items-05-10.md#m08-pr02), [M08-PR03](work-items-05-10.md#m08-pr03), [M08-PR04](work-items-05-10.md#m08-pr04), [M08-PR05](work-items-05-10.md#m08-pr05), [M08-PR06](work-items-05-10.md#m08-pr06) |
| M09 | Persistence Authority, WAL, Snapshot, and Recovery Format 2.0 | M02, M03, M04, M08-PR01 | [M09-PR01](work-items-05-10.md#m09-pr01), [M09-PR02](work-items-05-10.md#m09-pr02), [M09-PR03](work-items-05-10.md#m09-pr03), [M09-PR04](work-items-05-10.md#m09-pr04), [M09-PR05](work-items-05-10.md#m09-pr05), [M09-PR06](work-items-05-10.md#m09-pr06), [M09-PR07](work-items-05-10.md#m09-pr07), [M09-PR08](work-items-05-10.md#m09-pr08) |
| M10 | Native Capability Reintegration, Conformance Closure, and 2.0 Release | M01, M02, M03, M04, M05, M06, M07, M08, M09 | [M10-PR01](work-items-05-10.md#m10-pr01), [M10-PR02](work-items-05-10.md#m10-pr02), [M10-PR03](work-items-05-10.md#m10-pr03), [M10-PR04](work-items-05-10.md#m10-pr04), [M10-PR05](work-items-05-10.md#m10-pr05), [M10-PR06](work-items-05-10.md#m10-pr06), [M10-PR07](work-items-05-10.md#m10-pr07) |

## Critical path

`M00 → M01/M02 → M03 → M04 → M05 → M06 → M07 → M08 → M09 → M10`

<a id="m00"></a>
## M00 — 2.0 Pivot, 1.x End-of-Life, Governance, and Baselines

Create an irreversible and auditable line between the prototype-era 1.x series and the active 2.0 program before architectural code begins.

**Dependencies:** None

**Entry:**

- Confirm `b8782bec34ff0b815b62711ac7e33cac09d8ea71` is still the intended 1.x archival source snapshot.
- Confirm no active customer or internal deployment requires a 1.x patch stream or persisted-format migration.
- Ensure repository owner can create a protected archive branch and annotated non-release tag.

**Exit:**

- Repository-facing EOL language is unambiguous and repeated in README, CHANGELOG, contributing/agent guidance, and 2.0 docs.
- The version/branch/release policy cannot accidentally publish a 1.x or pre-release archive tag.
- The 2.0 plan, review protocol, baseline, and known-risk ledger are committed and linked from repository documentation.
- The complete baseline gate has been run on the pinned source, with failures and benchmark provenance recorded rather than normalized away.

<a id="m01"></a>
## M01 — Executable GQL Conformance Profile and Evidence System

Replace the hand-maintained feature and Annex B ledgers with one generated, implication-closed source of truth that controls claims, flagging, documentation, and tests.

**Dependencies:** M00

**Entry:**

- M00 EOL and repository governance are merged.
- The licensed ISO document remains local reference material and is not copied into the repository.
- The team accepts that the release claim is evidence-gated and may remain “ISO-aligned” until the generated gate is green.

**Exit:**

- Feature, implication, Annex B, Unicode, extension, and evidence registries are generated from one source.
- Current conflicting claims are either withdrawn or backed by scheduled implementation dependencies.
- Parser flagging, runtime feature reporting, docs, and release manifests consume the generated profile.
- A release claim gate can answer exactly which rule or evidence item blocks a claim.

<a id="m02"></a>
## M02 — Database Facade and Catalog Ownership

Move the public ownership root from one `SharedGraph` to an embedded `Database` whose catalog owns schemas, named graphs, graph types, procedures, constraints, and indexes.

**Dependencies:** M00, M01-PR01

**Entry:**

- The EOL policy explicitly permits removal of direct 1.x construction and session APIs.
- The catalog scope decision is locked: one synthetic root directory, multiple schemas, no child directories in 2.0.
- The implementation remains embedded and in-process; no server, wire protocol, or bundled authentication service is added.

**Exit:**

- An embedder can create/open a `Database`, create schemas and graphs, open a session, and execute against a named graph without constructing `SharedGraph` directly.
- Catalog descriptors have durable IDs, canonical names, parent ownership, object kind, and generation metadata.
- GQL `CREATE/DROP SCHEMA`, `CREATE/DROP GRAPH`, and `CREATE/DROP GRAPH TYPE` resolve through the same catalog service as Rust APIs.
- Old graph-root quickstarts and public entry points are removed rather than wrapped indefinitely.

<a id="m03"></a>
## M03 — Session, Request, Execution, and Transaction Semantics

Implement explicit ISO-shaped context and outcome objects so every request has deterministic schema, graph, parameter, transaction, diagnostic, and result semantics.

**Dependencies:** M02

**Entry:**

- Database/catalog identity and named graph lifecycle are merged.
- The implementation-defined profile has fields for default time zone, match mode, session parameters, principal behavior, and result-type exposure.
- A single-owner, allow-all local policy is acceptable as the default, while an embedder-supplied authorization provider remains possible.

**Exit:**

- Session set/reset/close and working schema/graph selection are testable across consecutive requests.
- Successful, omitted, no-data, warning, and failed outcomes preserve declared type and diagnostic bundles according to profile policy.
- Explicit START/COMMIT/ROLLBACK and implicit auto-commit produce equivalent state transitions where required.
- No request can outlive its session or observe a graph/catalog generation it was not analyzed against without a typed replan/retry outcome.

<a id="m04"></a>
## M04 — Stable Identity, Typed Candidate Sets, and Mixed Edge Directionality

Make physical row space impossible to misuse through public APIs and add explicit directed/undirected edge semantics across storage, traversal, type validation, and change events.

**Dependencies:** M02, M03-PR01

**Entry:**

- Named graph and catalog object identity is merged and available through the new database facade.
- The session root no longer borrows a single `SharedGraph`; graph selection is catalog-resolved.
- The team accepts removal of every public API that returns untyped row-space data.
- Logical identity and directionality records may be introduced now, while byte-level format 2.0 remains deferred to M09.

**Exit:**

- All public enumeration and index APIs yield IDs, typed handles, or `CandidateSet<Node/Edge>` rather than raw `RoaringBitmap`/`u32` rows.
- Set algebra refuses cross-graph, cross-generation, or cross-kind operations with typed errors.
- Edges carry explicit directionality and endpoint semantics, including loops and canonical undirected endpoint representation.
- Traversal and pattern matching distinguish edge directionality from path orientation.
- Change events and logical snapshot records can represent mixed edges without format ambiguity.

<a id="m05"></a>
## M05 — Semantic Compiler and Logical GQL IR

Separate source syntax, annotated semantics, logical algebra, and diagnostics so catalog resolution and GQL rules are explicit before physical planning.

**Dependencies:** M02, M03, M04-PR01

**Entry:**

- Catalog descriptors, named graph resolution, and explicit session/request contexts are merged.
- Stable node/edge/reference identity and mixed-edge semantic types are available to the analyzer.
- The generated conformance profile is the only authority for feature and implementation-defined behavior.
- The current parser corpus and analyzed-plan snapshots have a recorded baseline for differential work.

**Exit:**

- Every name, parameter, graph reference, property reference, variable, and procedure call has a resolved semantic descriptor or a structured diagnostic.
- Type/nullability/degree-of-reference and effect metadata are complete before lowering.
- Logical operators cover query, catalog mutation, data mutation, procedure, and transaction categories.
- Golden snapshots and deterministic IDs make analyzer/planner drift reviewable.
- The old mixed execution plan is isolated behind a temporary bridge with a scheduled deletion PR.

<a id="m06"></a>
## M06 — Physical Planning and Batch-Oriented Execution

Replace the row-at-a-time executor with a typed batch engine while preserving low-latency embedded use and exact GQL binding-table semantics.

**Dependencies:** M05, M04-PR02

**Entry:**

- A typed logical plan exists for every statement category that will enter the new executor.
- Public row-space leakage has been removed or isolated behind an internal compatibility bridge.
- The row-at-a-time executor has an explicit oracle role and differential corpus rather than an open-ended coexistence promise.
- Baseline latency, scan, join, grouping, and allocation rows are recorded before execution changes.

**Exit:**

- Scan, filter, project, join, group, sort, page, mutation, DDL, transaction, and procedure operators execute through the physical plan.
- Batch and row oracle outputs are differential-tested over the existing positive/negative corpus during transition.
- Result ordering, duplicates, nulls, record types, preferred column order, and status propagation match logical semantics.
- Performance gates cover one-row latency, medium scans, joins, grouping, and memory amplification.
- No public API exposes executor-internal column or row indices.

<a id="m07"></a>
## M07 — GQL Path Pattern and Selective Search Engine

Implement path expressions as explicit automata over mixed property graphs, with finite-result guarantees, path modes, match modes, selective search, and compact path values.

**Dependencies:** M05, M06-PR02, M04

**Entry:**

- Mixed directed/undirected edge semantics and stable graph-element references are merged.
- Path syntax and degree-of-reference information are represented in the annotated semantic tree and logical IR.
- The batch engine can stream typed binding tables and can host a dedicated path operator without exposing physical rows.
- A simple exhaustive small-graph path oracle and corpus format are agreed before optimized traversal begins.

**Exit:**

- Bounded patterns, quantified patterns, alternation, concatenation, parenthesized terms, and label/where constraints have explicit IR and tests.
- WALK, TRAIL, SIMPLE, ACYCLIC, DIFFERENT EDGES, and REPEATABLE ELEMENTS produce exact reference-equivalent results.
- ANY, ALL SHORTEST, counted shortest, and counted shortest-group selection are partitioned by endpoints as required.
- Path variables and path values preserve element order, edge orientation, and reference validity.
- Differential small-graph enumeration and targeted LDBC-style benchmarks gate optimization.

<a id="m08"></a>
## M08 — Catalog-Owned Constraints, Expression Indexes, JSON Paths, and Storage Performance

Unify integrity constraints and indexes under catalog ownership, make uniqueness incremental and composite, add deterministic expression indexes, and deliberately settle read-hot storage representations.

**Dependencies:** M02, M04, M05, M06-PR03

**Entry:**

- Catalog descriptor ownership and transaction-visible graph snapshots are merged.
- Typed candidate sets and generation checks are available for index access paths.
- Semantic scalar-expression descriptors and physical scan/filter operators exist.
- Current uniqueness, index drift, JSON scan, and read/write benchmark behavior has been captured before replacement.

**Exit:**

- Composite unique constraints are declarable for node and edge types and use typed tuple encoding without separator ambiguity.
- Incremental validation probes transaction-visible backing indexes and remains race-free under the single-writer boundary.
- Expression indexes are maintained on create/update/delete, replay, recovery, compaction, and schema change.
- The optimizer matches semantically equivalent JSON path expressions to registered targets.
- Read-hot map regressions are resolved or explicitly accepted with weighted workload evidence and permanent guard rows.

<a id="m09"></a>
## M09 — Persistence Authority, WAL, Snapshot, and Recovery Format 2.0

Establish one anchored filesystem and durability authority, implement explicit written/flushed/published/acknowledged commit phases, and cut over to a new 2.0-only persisted format.

**Dependencies:** M02, M03, M04, M08-PR01

**Entry:**

- Database/catalog ownership, transaction semantics, graph identity, mixed edges, and catalog object descriptors are stable enough to encode.
- Constraint and index descriptors have stable logical forms, even when derived accelerator bytes remain rebuildable.
- The final 1.x archival source has been protected and the no-migration policy is already public.
- A crash/fault-injection matrix and platform filesystem test strategy are approved before any format is declared authoritative.

**Exit:**

- Rename/replacement tests prove all WAL, manifest, lock, snapshot, archive, and temporary operations stay under the anchored directory.
- Written, flushed, published, and acknowledged positions are monotonic and crash-tested; rollback/cancel versus unknown outcomes are precise.
- Checksummed segment framing distinguishes provable tail tears from interior corruption without destructive repair.
- Snapshot/MANIFEST/CURRENT publication, checkpoint rotation, retention, and recovery form one state machine.
- Format 1.x stores fail with an actionable typed error and no files are modified.

<a id="m10"></a>
## M10 — Native Capability Reintegration, Conformance Closure, and 2.0 Release

Bring algorithms, vectors, text, JSON, and procedures back through the new facade and catalog, close the selected GQL profile with executable evidence, and harden a publishable 2.0 release.

**Dependencies:** M01, M02, M03, M04, M05, M06, M07, M08, M09

**Entry:**

- All preceding milestone exit gates are green at a pinned release-candidate base SHA.
- No temporary legacy public API, row executor, 1.x persistence reader, or dual ownership path remains in the stability-promised facade.
- The selected conformance profile and unresolved evidence gaps are visible and reviewed before release scope is frozen.
- Native feature baseline tests and benchmarks are available so reintegration can distinguish wiring changes from semantic drift.

**Exit:**

- Algorithms, vector, text, JSON, index maintenance, procedures, recovery, and catalog introspection work through `selene-db` and batch execution.
- The selected profile is implication-closed, all mandatory and claimed feature evidence is green, and Annex B/extension disclosures are generated.
- Public API, examples, rustdoc, error handling, release notes, package metadata, and crates.io dry runs are coherent.
- Full CI, fuzz, mutation, crash matrix, benchmark guard set, and cross-platform release workflows pass at the release candidate SHA.
- A final independent reviewer pair returns Blocker/Major-clean PASS on the unchanged exact head before an authorized merge; tag and publication remain separately controlled.
