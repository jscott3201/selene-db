# selene-db — repo guide for Claude

## Mission

`selene-db` is a strict ISO/IEC 39075:2024 GQL property graph engine, designed greenfield to keep the core small and high-performance and to move every non-graph capability (time-series, vectors, RDF, GraphRAG, etc.) into a clean extension/module system.

This is a **marathon, not a sprint**. No shortcuts. Every decision should optimize for correctness, performance, and a stable extension contract — even when that means more work today.

## Status

Active implementation. Architecture is settled (D1–D19, see Decision log). Workspace contains 5 of 7 v1.0 mandatory crates: `selene-core`, `selene-graph`, `selene-persist`, `selene-gql`, `selene-testing`. `selene-pack` and `selene-algorithms` remain to be built. Current milestone is **M5c** (planner + optimizer); see Milestone log below.

## North star: ISO/IEC 39075:2024

The ISO GQL spec PDF lives at `_spec/ISO_GQL/ISO_IEC_39075_2024(en).pdf` (~588 pages). Conformance is the language-level contract; everything else (transport, auth, extensions, isolation) is the implementation's choice.

Key facts:

- **One mandatory tier ("minimum conformance") + 228 optional features** (à la carte, not tiered profiles). Implications table (≈70 pairs) shows which features auto-imply others.
- **Mandatory data types are only `STRING / BOOLEAN / INT / FLOAT`.** Everything else is optional.
- **At least one of GG01 (open graph) or GG02 (closed graph) is required.**
- **Default isolation is serializable** (clause 4.6); relaxations are impl-defined (`IE004`).
- **No wire format is in the spec** (clause 4.2.3 is explicit).
- **Vectors, time-series, graph algorithms, auth — all out of spec.**
- Extension hooks the spec gives us: `IW010` (external procedures via normative `CALL`), `IV011` (dynamic property value type), `ID001`/`IW002`/`ID003` (impl-defined principals/authzn/privileges), `IE002`/`IE004` (transaction isolation).
- **GQL Flagger (clause 24.6) is required** because we ship optional features — non-standard constructs must syntactically flag at parse time. The canonical feature surface is `crates/selene-core/src/feature_register.rs::SUPPORTED_FEATURES`.

## Hard rules

1. **Strict ISO GQL compliance.** Anything not in the spec lives in extensions or impl-defined hooks. Never extend the GQL grammar itself.
2. **No shortcuts.** No "phase X scaffold," no placeholder framing, no TODO-it-later code. Documents describe the layer the type ships, not the order of sprints.
3. **`#![forbid(unsafe_code)]` workspace-wide.** Bounded RFC-specified surfaces may be exceptions, with justification in `// Why:` comments.
4. **`missing_docs = "deny"` workspace-wide.** Every `pub` item carries a rustdoc with intent, invariants, or non-obvious behavior. Dead surfaces get demoted to `pub(crate)`, not stubbed.
5. **File-size cap: 700 LOC.** Refactor or split before approaching the cap. Enforced by `.github/scripts/check-file-size.sh`.
6. **Per-crate LOC budgets:** `selene-core` ≤ 3K; `selene-graph` ≤ 8K; `selene-gql` ≤ 35–45K; `selene-persist` ≤ 5K; `selene-pack` ≤ 10K; `selene-algorithms` ≤ 5K; `selene-testing` ≤ 2K. Anything beyond a budget moves to a module/extension crate.
7. **rustls-only TLS posture.** No native-tls, no openssl-sys. Enforced in `deny.toml`.
8. **Dual MIT OR Apache-2.0 license.** Permissive transitive license allow-list per `deny.toml` (with one explicit `MPL-2.0` exception for `imbl`, see D7).
9. **Latest stable Rust.** Pinned in `rust-toolchain.toml`. Latest published deps with strong adoption signals.
10. **Never hand-roll crypto / TLS / async runtime / serialization primitives.**
11. **Every new I/O surface routes through a single mutation/auth funnel — enforced by types.** No "this new endpoint forgot to go through the funnel" class of bug.
12. **GQL is the sole query and mutation interface.** No SQL/Cypher/SPARQL grammar in the engine; if needed, ship as extensions that translate to GQL or call procedures.
13. **Attribution.** Donor codebases (SeleneDB, AetherDB, aether-db, rusty-bacnet) are the user's own work and need no per-line attribution. Third-party code (crates, blog posts, vendored snippets) does:
    - `NOTICE` — Apache-2.0-style notice naming third-party copyright holders for bundled/adapted code.
    - `THIRDPARTY.md` — auto-generated from `Cargo.lock` via `cargo-about`; CI-gated against drift.
    - **Per-file attribution** for non-trivial adapted third-party code: `// Adapted from <upstream>@<version-or-commit> (<SPDX>) — <brief note>`.
    - Spec derivation: cite ISO clause numbers in `// Why:` comments (`// Per ISO 39075:2024 clause 16.4`).

## Forward-looking principles (from donor archaeology)

- **Make the extension boundary first-class on day one.** SeleneDB's `Procedure::execute(...hot_tier: Option<&HotTier>...)` was the most damaging coupling — every procedure carried a TS reference because the trait was designed when TS was first-class. The fix is **per-tier `Context` types + per-tier registries** (D17).
- **Refuse to put auxiliary subsystems inside core.** HNSW lived inside `selene-graph` and that's why "core" grew to 19 KLOC. Vectors, time-series, RDF, GraphRAG all live as extensions (D5).
- **Two-stage validation with explicit coverage struct** (the aether-db pattern). Make "what we check today" vs "what the spec demands" a programmatic value, not a comment.
- **Typestate-sealed registry transitions + atomic graph commit + post-commit audit.** Audit lag is recoverable; fictional audit is not.

## Crate map

v1.0 mandatory (every consumer pulls these), with dependency direction:

| Crate | Depends on | Owns |
|---|---|---|
| `selene-core` | none | Foundation types: `Value` (4 mandatory ISO types + `Value::Extended`), `IStr`, `PropertyMap`, `LabelSet`, schema types, `Codec`, `Origin`, `Changeset`. |
| `selene-graph` | core | In-memory property graph: ArcSwap+RwLock+imbl, RoaringBitmap label index, TypedIndex, `IndexProvider` trait, `Mutator` write funnel, `GraphTypeDef` runtime binding. |
| `selene-persist` | core | WAL (`SLDB` magic) + snapshot (`SLSN` TLV-tagged sections) + recovery. **Never sees `Graph`** — takes `&[Change]`, returns `RecoveryResult`. |
| `selene-gql` | core, graph | ISO GQL parser (pest), AST, semantic analyzer, planner, optimizer, executor, `ProcedureRegistry` trait. |
| `selene-pack` (TBD) | core, graph, gql | Procedure-pack registry, validator, activation state machine, native procedure runtime. |
| `selene-algorithms` (TBD) | core, graph | Graph algorithms (PageRank, WCC, SCC, Dijkstra, etc.); independent of GQL. |
| `selene-testing` | core, graph | Test fixtures and synthetic graph generators. |

Opt-in extension crates depend on the mandatory crates plus the procedure-pack/index hooks: `selene-vector` (HNSW + vector procedures, D5); future `selene-timeseries`, `selene-rdf`, `selene-graphrag`, `selene-fulltext`. **No umbrella crate.** Crate boundaries are enforced by code review and `cargo-deny`.

## Decision log

All v1.0 architecture is settled. Canonical text lives in the linked spec section; this table is the breadcrumb index. Fields named in a row reflect the *current* shape after any amendments.

| ID | Decision | Canonical home |
|---|---|---|
| D1 | v1.0 = embeddable library only (no server, no transport, no auth, no MCP) | `_spec/01` §4 |
| D2 | Conformance: minimum + ~30–40 parser-reachable optional features | `_spec/01` §5 |
| D3 | Schema: both GG01 (open) and GG02 (closed); per-graph choice | `_spec/02` §6 |
| D4 | Procedure-pack extension model + feature-gated workspace modules | `_spec/05` §3, §5 |
| D5 | Vector via `Value::Extended`; HNSW lives in `selene-vector` | `_spec/02` §3, `_spec/06` §3 |
| D6 | WAL+snapshot from day one in `selene-persist` | `_spec/04` |
| D7 | Concurrency: `ArcSwap` + `parking_lot::RwLock` + `imbl` CoW; `imbl` MPL-2.0 exception accepted | `_spec/03` §4 |
| D8 | Multi-crate, no umbrella; linear dep direction | this file (Crate map), `_spec/03` §8 |
| D9 | Workspace dependency choices (imbl, rkyv, postcard, blake3, jiff, …) | `_spec/09` §3 |
| D10 | Transaction isolation: single graph write-lock + lock-free reads (strict-serializable) | `_spec/03` §6 |
| D11 | ID allocation: aborted-tx IDs become permanent holes | `_spec/02` §4, `_spec/03` §6 |
| D12 | Audit: opaque caller-supplied principal byte-slot in WAL header | `_spec/04` §3 |
| D13 | Snapshot section tag: 8-byte composite (provider + sub-tag); 24-byte row | `_spec/04` §4 |
| D14 | Snapshot serialization: `rkyv` archive over sorted-vec intermediates | `_spec/04` §4 |
| D15 | Recovery: two-step (snapshot apply → WAL replay); `RecoveryProvider` lives in `selene-persist`, `IndexProvider` lives in `selene-graph`; both use `&self` receivers | `_spec/06` §3, §12 |
| D16 | `ProcedureRegistry` trait owned by `selene-gql`; `lookup` takes `&[IStr]`; embedder injects `&dyn ProcedureRegistry` | `_spec/08` §7 |
| D17 | Procedure tiers: per-tier concrete `Context` structs + per-tier dyn-compatible `Procedure` traits | `_spec/05` §3 |
| D18 | Pack lifecycle audit: WAL-only via the same mutation funnel; no parallel ledger | `_spec/05` §5 |
| D19 | GG02 catalog: per-graph immutable `bound_type: Option<Arc<GraphTypeDef>>` runtime binding, persisted via `CORE/GTYP` | `_spec/02` §6, `_spec/03` §3 |

## Milestone log

- **M5b closed (2026-05-09)** — analyzer entry: `analyze(stmt, &registry, schema)` produces binding scopes, expression type cells, procedure/YIELD typing, statement category, mutation write sets, and statically decidable closed-graph schema errors. Runtime `selene_graph::type_validator` remains the commit-time backstop.
- **M5c opened (2026-05-09)** — lifts `AnalyzedStatement` into an optimized `ExecutionPlan`. Spec 13 (`_spec/13-iso-gql-planner.md`, local-only mirror) records implementation invariants. BRIEF-26 landed Plan IR + read-pipeline lowering; BRIEF-27 lowers mutations/DDL/CALL/transactions; BRIEFs 28–29 add the optimizer; BRIEF-30 closes M5c with the plan snapshot harness.
- **M5c progress 2/5 (2026-05-09)** — BRIEF-27 closed mutation, DDL, CALL, and transaction-control lowering. Planner output is now complete for every v1.0-claimed statement shape (no `NotImplemented` hatches at the top-level dispatch). Adds 5 new `PlannerError` variants (`SLENE_P_013..017`) that defend against analyze→plan registry drift and interner cap exhaustion. BRIEFs 28–29 add the optimizer next.
- **M5c progress 3/5 (2026-05-09)** — BRIEF-28 closed the optimizer framework + 5 structural rules (ConstantFolding, AndSplitting, FilterPushdown, ExpandFilterPushdown, TopK). `Rule` trait is infallible; `OptimizeContext` carries `EdgeStatistics` + `WanderJoinSampler` skeletons (cost-aware activation in BRIEF-29). `walk_expand_nodes` skips `JoinTree::Outer.right` to preserve post-OPTIONAL FILTER semantics. `#[non_exhaustive]` on `OptimizeContext`/`EdgeStatistics`/`PropertyHistogram` reserves room for BRIEF-29's `IndexCatalog` + selectivity hooks. BRIEF-29 lands the remaining 8 rules next.
- **M5c progress 4/5 (2026-05-09)** — BRIEF-29 closed the remaining 8 optimizer rules to complete the v1.0 13-rule set: `NodeFilterExtraction`, `CompositeIndexLookup` (with subset matching via Gosper's hack), `InListOptimization`, `RangeIndexScan` (with tightest-bound merging + contradiction detection), `IndexOrder`, `PredicateReorder`, `WcoJoin` (marker-only; AGM math is M5d Phase B), `SymmetryBreaking`. Adds `IndexCatalog` trait (D16-mirror, embedder-injected) with `IndexTarget::{Node, Edge}` and opaque `IndexHandle`; `ScanAccess` / `OrderAccess` / `TypedIndexBounds` / `NodeIdOrdering` IR additions in `plan/ir/access.rs`; `ExecutionPlan.next_expr_id` allocator; `#[non_exhaustive]` on `JoinTree`, `ImplDefinedCaps`, `NodeIdOrdering`. v1.0 edge typed/composite indexes always return `Linear` (selene-graph's typed property index is node-only — embedder concern for v1.x). BRIEF-30 closes M5c with the plan snapshot harness next.
- **M5c CLOSED (2026-05-09)** — BRIEF-30 (PR #34, merge `f6590a1`) landed the plan snapshot corpus harness behind a `test-harness` feature flag (`[[test]] required-features = ["test-harness"]` so the snapshot test isolates from `cargo test --workspace --no-default-features`). Adds `selene_testing::PlanCorpus` (18 entries × stable slug, category, expected rules; `PlanCorpusRegistry::{Empty, StandardMock}` selector) + `optimize_summary` rendering layer (`PlanSnapshot` / `PatternSnapshot` / `ScanSnapshot` / `PipelineOpSummary`, gated `#[cfg(any(test, feature = "test-harness"))]`). Three RULE_NAMES drift checks at distinct layers (registry-internal, corpus-known, snapshot-fired) make the system self-policing. Three type-preservation tests cover surviving expression rows, synthesized-predicate booleanness (using `analyzed.expr_types.len()` as the high-water mark for ExprId allocator coverage), and binding-ref sortedness/dedupe/pattern-resolvability. Spec 13 §7 (10 subsections) records optimizer implementation invariants in the local-only `_spec/13-iso-gql-planner.md` mirror. Rolled-in scope: lexer-aware parser nesting guard at depth 64 (`parser/guard.rs`) with `ParserError::NestingLimitExceeded` (SLENE_GQL_54000) — resolves a parse-fuzz timeout artifact found during validation; covered by `dos_guard.rs` (rejects over-nested input, ignores delimiters in strings/comments). M5c's planner+optimizer is now a closed, snapshot-protected surface; M5d (executor) opens next.

## Build & test

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --all-features
cargo deny check bans licenses sources
cargo audit -d /private/tmp/selene-advisory-db
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
bash .github/scripts/check-thirdparty-current.sh
cargo +nightly fuzz run parse_gql -- -max_total_time=60
```

## Conventions

- **Commits:** conventional commits (`feat(scope):`, `fix(scope):`, `refactor(scope):`); scope = crate or component.
- **Branches:** trunk + `feature/*` PRs against `main`.
- **Tests with code:** every PR ships with extensive tests — units, edge cases, error paths, concurrency for shared state, property tests for invariants. Bar is "would this catch the IStr admission race."
- **Decisions over guesses:** when blocked, surface the question with options and a recommendation. Don't pre-commit.

## Working with the user

- The user is the sole author/owner of all donor codebases (no third-party license entanglements).
- Questions one at a time, with discrete options and a recommendation.
- Marathon mindset: prefer the most robust/performant/non-tech-debt path even when it touches many areas.
- This repo stays private on GitHub for the foreseeable future.

## Donor codebases (guidance only — no dependencies)

The following codebases are the user's, MIT/Apache-licensed, and are donors of design lessons and code — **not runtime dependencies**:

| Path | What it is | What we mine |
|---|---|---|
| `/Users/justin/Development/SeleneDB/` | Original prototype (~141 KLOC, 13 crates) | Battle-tested core types, pest GQL grammar, ArcSwap+RwLock concurrency, WAL/snapshot shape, RoaringBitmap label indexes. **Cautionary tales:** server bloat, HNSW inside graph crate, GQL→TS coupling, 7 critical authz bugs. |
| `/Users/justin/Development/AetherDB/` | First fork; cut RDF/TS/CLI/HTTP/MCP/federation/vault/OAuth | First scope-reduction attempt; still kept HNSW in core. |
| `/Users/justin/Development/AgentAether/aether-db/` | Second, library-only fork (7 crates, ~61 KLOC) | **Most refined extension model in the lineage.** Procedure-pack: JSON manifest, typestate-sealed activation, atomic graph-commit + post-commit audit, native `fn(&Value) -> ProcedureExecutionResult` adapters with JSON Schema gates. |
| `/Users/justin/Development/rusty-bacnet/` | Codec/parser donor (same author, MIT) | Codec patterns; surveyed 2026-05-08. |

Detailed research notes are in repo memory (`project_lessons_from_archaeology.md`, `project_donor_perf_audit.md`).

## Templates

Floor-level CI/license/lint scaffolding lives at `/Users/justin/Development/AgentAether/_templates/`:

- `ci/rust-baseline.yml` — fmt / clippy `-D warnings` / test / cargo-deny / cargo-audit / file-size cap / no-secret scan
- `ci/deny.toml` — rustls-only, MIT/Apache-2.0 + permissive transitive licenses, crates.io-only sources
- `ci/rust-toolchain.toml` — stable + rustfmt + clippy
- `ci/rustfmt.toml` — edition 2024, max_width 100
- `ci/scripts/check-file-size.sh` — 700 LOC cap
- `ci/scripts/check-no-secrets.sh` — baseline secret-pattern grep

These are the floor; per-repo additions tighten but don't weaken.

## Underscore-folder convention (working documents, not committed)

Top-level directories beginning with `_` (e.g., `_spec/`, `_briefs/`, `_review/`, `_design/`, `_plan/`) are **local-only working documents** and excluded from git via `.gitignore`'s `_*/` pattern.

Canonical durable state for design decisions, milestones, work items, and prior art lives in the **MCP graph** (selenedb instance — project node 364, label `selene-db`). The underscore-prefixed folders are dense markdown views of that state plus working scratchpads (briefs to agents, review reports, draft specs).

Implications:

- Tracked files (Cargo.toml, source under `crates/`, CLAUDE.md, LICENSE-*, NOTICE, THIRDPARTY.md, deny.toml, CI workflows, etc.) ship in git.
- Untracked working documents (`_spec/*`, `_briefs/*`, `_review/*`, `_design/*`, `_plan/*`) live on the local filesystem and are reproducible from the graph.
- When briefing an agent (Codex, etc.), specs are referenced via absolute filesystem paths, NOT relative-to-checkout paths, because a fresh `git clone` doesn't include them.
- The MCP graph is the source of truth. If a working document drifts from the graph, the graph wins; regenerate the working document.
