# AGENTS.md - selene-db

Canonical startup context for AI coding agents working in this repository.
`CLAUDE.md` is a symlink to this file, so this is the repo-specific source of
truth for both Codex and Claude.

The universal partner workflow lives outside the repo:

- Executor condensation: `~/.codex/AGENTS.md`
- Full lead workflow: `/Users/justin/Development/_agent_helpers/partner-workflow.md`

This file is intentionally repo-specific. It should describe the current engine,
validation gates, and local constraints. It should not carry fast-moving PR
state, stale release plans, or long historical decision tables.

## Mission

`selene-db` is a greenfield, embeddable Rust graph database centered on
ISO/IEC 39075:2024 GQL. It is a single native engine:

- GQL is the only query and mutation language.
- Graph algorithms are mandatory native engine capability, not extensions.
- Dense vectors are first-class engine values and vector indexes are native graph
  indexes.
- Native platform and algorithm surfaces are exposed through `CALL selene.*` and
  `CALL algo.*`, not through grammar drift.
- There is no loadable extension pack system.

The current workspace package version is `1.1.0`; Rust is edition 2024 with the
toolchain pinned to `1.95.0`.

## Non-Negotiables

1. Keep ISO GQL compliance at the language boundary. Non-standard behavior must
   live behind implementation-defined types, values, or procedures and must be
   feature-flagged where the parser exposes it.
2. Do not add SQL, Cypher, SPARQL, or ad hoc query grammar.
3. Do not bypass the mutation funnel. Schema/index writes go through the
   `Mutator` path; maintenance writes use the maintenance context; read-only
   built-ins must not mutate or re-enter writes.
4. Keep `#![forbid(unsafe_code)]` and `missing_docs = "deny"` working
   workspace-wide. Any future unsafe exception needs an explicit, reviewed
   architectural reason.
5. Keep files under the 700 LOC cap. Split modules before they approach the cap.
6. Prefer no new dependencies. When a dependency is justified, it must be
   current, maintained, license-compatible, and stronger than a local
   implementation. SIMD/NEON/acceleration crates are acceptable when well
   supported.
7. Do not hand-roll crypto, TLS, async runtimes, or serialization primitives.
8. Keep the rustls-only posture. No native-tls, no openssl-sys.
9. Preserve the dual MIT OR Apache-2.0 license posture and third-party
   attribution (`NOTICE`, `THIRDPARTY.md`, per-file attribution for adapted
   third-party code).
10. Treat benchmark evidence as part of engineering, not decoration. New
    performance claims need commands, scales, and numbers.

Error codes are GQLSTATUS, not SQLSTATE. Do not introduce SQL drift such as
`LIKE`, `BETWEEN`, `^`, `XX500`, `0A000`, `42883`, or `22023` unless the current
feature register and parser explicitly support the GQL construct.

## Workspace Map

The workspace has no umbrella crate. Dependency direction is intentionally
linear:

`selene-core -> selene-graph -> selene-algorithms -> selene-gql`

`selene-persist` depends on `selene-core` and stays below `selene-graph`.
`selene-testing` is fixtures and harness support, normally dev-dependency only.

| Crate | Owns |
|---|---|
| `selene-core` | Foundation types: `Value`, `VectorValue`, vector metrics/top-k helpers, `IStr`, identities, schema/value types, feature register, property maps, codecs, changesets. |
| `selene-graph` | In-memory graph storage, `SharedGraph`, `Mutator`, row/id maps, property/composite indexes, vector indexes, exact/ANN/candidate vector search, recovery provider, compaction, graph type enforcement. |
| `selene-persist` | WAL, snapshots, MANIFEST recovery, audit log, retention/prune. It does not own graph semantics. |
| `selene-algorithms` | Projection catalog plus native structural, pathfinding, centrality, and community algorithms. It depends on core/graph only and never on GQL. |
| `selene-gql` | Parser, AST, analyzer, planner, optimizer, executor, procedure tiers, and the concrete native `BuiltinProcedureRegistry`. |
| `selene-testing` | Shared test fixtures, graph generators, benchmark profiles, snapshot-harness support. |

## Current Native Surfaces

### GQL And Procedures

- `crates/selene-core/src/feature_register.rs` is the canonical optional-feature
  surface.
- `BuiltinProcedureRegistry` currently exposes 37 procedures: 18 `selene.*`
  platform built-ins plus 19 `algo.*` graph algorithm procedures.
- The registry is concrete and native. The `ProcedureRegistry` trait remains the
  planner/executor/test seam, not a third-party extension point.
- Procedure tiering matters:
  - Graph tier: read-only health, feature status, verify, vector search/score,
    vector index stats.
  - Mutation tier: property index and vector index create/drop.
  - Maintenance tier: vector index rebuild/recommended rebuild.

### Vectors

Vectors are first-class, not externalized:

- `Value::Vector(VectorValue)` is the native dense-vector value.
- `VectorValue` stores finite, non-empty `f32` components behind shared storage.
- `MAX_VECTOR_DIMENSION` is `u16::MAX`.
- Metrics are exact lower-is-better `squared_euclidean`, `cosine`, and
  `negative_inner_product`.
- Core vector kernels use safe SIMD through `wide` where available and keep
  `f64` score semantics.

Vector indexes are native graph indexes over `(label, property)`:

- `Flat`
- `HnswSquaredEuclidean`, `HnswCosine`, `HnswNegativeInnerProduct`
- `IvfSquaredEuclidean`, `IvfCosine`, `IvfNegativeInnerProduct`

Index registrations are durable schema state; HNSW/IVF accelerators are derived
in-memory state and can be rebuilt from primary graph values. Delete/update
visibility and rebuild correctness are required tests for any vector-index
change.

Production vector APIs include:

- exact node search and batch exact search;
- ANN node search and batch ANN search;
- explicit candidate scoring and batch candidate scoring;
- one-hop graph-neighbor scoring and batch neighbor scoring;
- index stats, create/drop, rebuild, and recommended rebuild.

Research-only vector compression/ANN experiments live in benchmark code until
they earn a production design. PQ, IVF+PQ, binary sign-bit scoring, OPQ, ScaNN,
DiskANN, and TurboQuant-style ideas are benchmark/research inputs, not
production surfaces by default.

### Graph Algorithms

`selene-algorithms` is mandatory and native. Algorithms operate on
`GraphProjection`, not directly on live mutable graph state. Keep algorithm
work independent of `selene-gql`; GQL bindings are registry adapters over the
native Rust API.

### Graph-Accelerated Retrieval

Agentic-memory workloads are a major product driver, but the engine should ship
policy-neutral primitives. Prefer composable pieces:

- graph-derived candidate sets;
- exact vector scoring over explicit candidates;
- graph-neighbor vector scoring;
- maintained or rebuildable active sets when evidence supports them;
- graph algorithm priors only when they change candidate coverage, diversity, or
  correctness.

Do not hard-code one agent-memory retrieval policy into the engine.

### BM25 / Full Text

Native BM25/full-text search is queued research, not production code yet. When
it starts, treat it like vectors and algorithms:

- ISO GQL grammar stays strict.
- Expose through native values/indexes/procedures as appropriate.
- Ground against Tantivy and the old local SeleneDB donor code, but do not add a
  dependency or copy old code without fresh design, tests, and benchmarks.
- Define delete/update/WAL/snapshot/rebuild semantics before shipping.
- Test hybrid graph/vector/text retrieval, not only text-only ranking.

## Performance Posture

Expected workload shape is read-heavy but not append-only: roughly 60% reads and
40% writes. Optimize for evidence:

- Use Rayon when the workload size justifies parallel overhead.
- Use safe SIMD/NEON acceleration where stable and measurable.
- Keep library crates allocator-agnostic. Bench binaries use mimalloc; a global
  engine allocator needs benchmark evidence and an explicit decision.
- WAL durability is often the write-side cost center. Do not optimize provider
  or index-maintenance micro-paths past the evidence.
- Prefer larger, product-shaped benchmark rows over tiny loop tuning once the
  API boundary is correct.

## Build And Validation

Use the repo scripts rather than recreating CI inline.

Common local gate:

```bash
cargo fmt --all --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --all-features --profile default
cargo test --workspace --locked --all-features --doc
cargo deny check bans licenses sources
cargo audit -d /private/tmp/selene-advisory-db
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
bash .github/scripts/check-thirdparty-current.sh
bash .github/scripts/check-no-rowid-arith.sh
bash .github/scripts/check-no-version-locked-feature-error.sh
```

Fuzz when parser or persist decoding risk changed:

```bash
cd crates/selene-gql/fuzz
cargo +nightly fuzz run parse_gql -- -max_total_time=60
```

Docs-only changes can run the cheap local gates, but any code change needs the
relevant focused tests plus the full gate before PR.

## Benchmarks

`scripts/run-benches.sh` is the only sanctioned benchmark entry point. Do not run
`cargo bench --workspace`; Cargo may run bench binaries concurrently and pollute
wall-clock medians.

Useful invocations:

```bash
scripts/run-benches.sh --list
scripts/run-benches.sh --smoke
scripts/run-benches.sh --profile quick --bench vector_graph_retrieval
scripts/run-benches.sh --profile quick --bench procedure_call_repeat --filter vector
scripts/run-benches.sh --profile quick --bench vector_index_rebuild --vector-scales 10000
scripts/run-benches.sh --bench vector_index_rebuild --allocator system
```

The benchmark suite is Criterion-only. iai-callgrind/valgrind is not part of
the current runner. Every committed bench target must be registered in
`scripts/run-benches.sh` and documented in `BENCHMARKS.md`; CI checks both.

## CI And Hooks

- PRs to `development` run the cheap `ci.yml` gate: rustfmt, file-size,
  no-secrets, row-id arithmetic, version-locked feature error, benchmark
  invocation/docs, and dependency gates only when manifests changed.
- PRs to `main` run `release.yml`: clippy, nextest, doctests, deny, audit,
  third-party attribution, macOS validation, parser fuzz, and persist fuzz.
- Nightly handles longer advisory/fuzz work.
- `.githooks/pre-commit` mirrors cheap checks.
- `.githooks/pre-push` runs fast workspace clippy.

Install hooks with:

```bash
scripts/install-hooks.sh
```

After each merged PR in the long-running goal workflow, run:

```bash
cargo clean
```

## Branches And PRs

- `development` is the integration trunk.
- Release PRs go from `development` to `main`.
- Use conventional commits with a meaningful scope: `feat(vector): ...`,
  `bench(algorithms): ...`, `docs(workflow): ...`.
- Keep PRs focused. One behavior, benchmark slice, or docs refresh per PR.
- Use GitHub connector tools when available for PR comments, CI polling, and
  merges. Merge authority is governed by the current user/lead instructions, not
  by this file.
- Do not mention the cloud reviewer in PR bodies or comments with trigger
  syntax. Do not add reactions.

## Local Working Docs

Top-level underscore directories are local-only and gitignored:

- `_briefs/`, `_design/`, `_review/`, `_spec/`, `_plan/`, `_goalslogs/`

Never commit them. They are working views, scratchpads, or local goal memory.
For this long-running vector/performance goal, `_goalslogs/` is the preferred
place to keep local research notes, PR logs, benchmark ledgers, and follow-up
queues.

Tracked docs such as `AGENTS.md`, `BENCHMARKS.md`, `NOTICE`,
`THIRDPARTY.md`, workflow files, scripts, and crate docs are normal repo
artifacts and may be committed.

## Donor Code

The user owns the donor codebases below. They are reference material, not
runtime dependencies:

| Path | Use |
|---|---|
| `/Users/justin/Development/SeleneDB/` | Original prototype. Useful for vector/BM25 archaeology and cautionary examples. |
| `/Users/justin/Development/AetherDB/` | Scope-reduction fork and prior storage/API lessons. |
| `/Users/justin/Development/AgentAether/aether-db/` | Library-only fork; useful for audit/funnel patterns, not extension-pack revival. |
| `/Users/justin/Development/rusty-bacnet/` | Codec/parser donor patterns. |

Archived MCP services and old donor code are not authoritative for this repo.
Current source, tests, benchmarks, and GitHub state win.

## Recurring Footguns

- Do not re-externalize vectors or describe them as out-of-tree. They are native
  `Value` and graph-index functionality now.
- Do not revive procedure packs, manifest validation, or loadable extensions.
- Do not add BM25/full-text as a grammar shortcut or dependency-first import.
- Do not commit `_goalslogs` or any other `_*/` working directory.
- Do not bypass row-id mapping. External `NodeId`/`EdgeId` are stable; internal
  `RowIndex` is a storage position.
- Do not skip benchmark documentation for new benchmark targets.
- Do not optimize beyond the current evidence trail; queue research when the
  next step needs a larger design.
