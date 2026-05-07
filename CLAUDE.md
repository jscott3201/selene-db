# selene-db — repo guide for Claude

## Mission

`selene-db` is a strict ISO/IEC 39075:2024 GQL property graph engine, designed greenfield to keep the core small and high-performance and to move every non-graph capability (time-series, vectors, RDF, GraphRAG, etc.) into a clean extension/module system.

This is a **marathon, not a sprint**. No shortcuts. Every decision should optimize for correctness, performance, and a stable extension contract — even when that means more work today.

## Status (2026-05-07)

Greenfield. Research phase complete; brainstorming phase active. **Architecture decisions are still being settled — see "Open design questions" below before assuming structure.** Floor-level scaffolding will land after the brainstorming pass agrees on identity, extension model, and conformance scope.

## North star: ISO/IEC 39075:2024

The ISO GQL spec PDF lives at `_spec/ISO_GQL/ISO_IEC_39075_2024(en).pdf` (~588 pages). Conformance is the language-level contract; everything else (transport, auth, extensions, isolation) is the implementation's choice.

Key facts (derived 2026-05-07; full notes in repo memory):

- **One mandatory tier ("minimum conformance") + 228 optional features** (à la carte, not tiered profiles). Implications table (≈70 pairs) shows which features auto-imply others.
- **Mandatory data types are only `STRING / BOOLEAN / INT / FLOAT`.** Everything else (sized ints, DECIMAL, temporal types, LIST, PATH, RECORD, reference types) is optional.
- **At least one of GG01 (open graph / schemaless) or GG02 (closed graph / typed) is required.**
- **Default isolation is serializable** (clause 4.6); relaxations are implementation-defined (`IE004`).
- **No wire format is in the spec** (clause 4.2.3 is explicit).
- **Vectors, time-series, graph algorithms, auth — all out of spec.**
- Extension hooks the spec gives us:
  - `IW010` — external procedures mechanism (how user-defined procedures are registered; invoked via normative `CALL` clause).
  - `IV011` — dynamic property value type (the union backing unspecified properties).
  - `ID001` / `IW002` / `ID003` — principals, authorization identifiers, privilege set (entire security model is impl-defined).
  - `IE002` / `IE004` — transaction isolation level configuration.
- **GQL Flagger (clause 24.6) is required if any extension or optional feature is offered** — must syntactically flag non-standard constructs.

## Hard rules

1. **Strict ISO GQL compliance.** Anything not in the spec lives in extensions or impl-defined hooks. We never extend the GQL grammar itself.
2. **No shortcuts.** No "phase X scaffold," no placeholder framing, no TODO-it-later code. Documents describe the layer the type ships, not the order of sprints.
3. **`#![forbid(unsafe_code)]` workspace-wide.** Bounded RFC-specified surfaces may be exceptions, with justification in `// Why:` comments.
4. **`missing_docs = "deny"` workspace-wide.** Every `pub` item carries a rustdoc with intent, invariants, or non-obvious behavior. Dead surfaces get demoted to `pub(crate)`, not stubbed.
5. **File-size cap: 700 LOC.** Refactor or split before approaching the cap. Enforced in CI by `.github/scripts/check-file-size.sh`.
6. **Per-crate LOC budgets:** `selene-core` ≤ 3K; the property-graph crate ≤ 8K; the GQL crate ≤ 35–45K. Anything beyond a budget moves to a module/extension crate.
7. **rustls-only TLS posture.** No native-tls, no openssl-sys. Enforced in `deny.toml`.
8. **Dual MIT OR Apache-2.0 license.** Permissive transitive license allow-list per template `deny.toml`.
9. **Latest stable Rust.** Pinned in `rust-toolchain.toml`. Latest published deps with strong adoption signals.
10. **Never hand-roll crypto / TLS / async runtime / serialization primitives.**
11. **Every new I/O surface routes through a single mutation/auth funnel — enforced by types.** No "this new endpoint forgot to go through the funnel" class of bug.
12. **GQL is the sole query and mutation interface.** No SQL/Cypher/SPARQL grammar in the engine itself; if those are needed, they ship as extensions that translate to GQL or call procedures.
13. **Attribute every line of third-party code.** The donor codebases (SeleneDB, AetherDB, aether-db) are the user's own work and need no attribution — code can flow in freely without provenance comments. Third-party code (anything from a crate, blog post, paper, vendored snippet) is a different story:
    - `NOTICE` at the repo root — Apache-2.0-style notice file naming third-party copyright holders for any code we bundle or adapt at the source level (vendored snippets, ported algorithms with non-trivial provenance).
    - `THIRDPARTY.md` at the repo root — auto-generated from `Cargo.lock` via `cargo-about` (or equivalent), listing every transitive dependency with its license and SPDX identifier. Regenerated in CI; drift is a merge block.
    - **Per-file attribution comments** for any non-trivial code adapted from a third-party source (not donor codebases). Format: `// Adapted from <upstream>@<version-or-commit> (<SPDX>) — <brief note>`. Lives at the top of the file alongside the rustdoc.
    - For grammar / semantics derived from the ISO/IEC 39075:2024 spec: cite clause numbers in `// Why:` comments (`// Per ISO 39075:2024 clause 16.4`).
    - Rationale: legal compliance with permissive licenses (MIT/Apache require notice preservation; MPL-2.0's file-level copyleft for `imbl` requires distributing the imbl license alongside any redistribution); clean provenance trail for future maintainers; no surprises on a downstream license audit.

## Forward-looking principles (from donor archaeology)

- **Make the extension boundary first-class on day one.** The single most damaging coupling in SeleneDB was `Procedure::execute(...hot_tier: Option<&HotTier>...)` — every procedure carried a TS reference because the trait was designed when TS was first-class. The right answer is a **generic `Context` type per registry tier** plus per-tier registries. Trait providers via `OnceLock` static registration (the pattern that worked for `RdfProvider` and `VectorProvider`) is the seam to replicate.
- **Refuse to put auxiliary subsystems inside core.** HNSW lived inside `selene-graph` and that's why "core" grew to 19 KLOC. Vectors, time-series, RDF, GraphRAG all live as extensions.
- **Two-stage validation with explicit coverage struct** (the aether-db pattern). Make "what we check today" vs "what the spec demands" a programmatic value, not a comment.
- **Typestate-sealed registry transitions + atomic graph commit + post-commit audit.** Audit lag is recoverable; fictional audit is not.

## Donor codebases (guidance only — no dependencies)

The following codebases are mine, MIT/Apache-licensed, and will be retired/archived. They are donors of design lessons and code, **not runtime dependencies of selene-db**:

| Path | What it is | What we mine from it |
|---|---|---|
| `/Users/justin/Development/SeleneDB/` | Original prototype (~141 KLOC, 13 crates) | Battle-tested core types, pest GQL grammar, ArcSwap+RwLock concurrency, WAL/snapshot persistence shape, RoaringBitmap label indexes. **Cautionary tales:** server bloat (45 KLOC), HNSW inside graph crate, GQL→TS coupling, 7 critical authz bugs. |
| `/Users/justin/Development/AetherDB/` | First fork; cut RDF/TS/CLI/HTTP/MCP/federation/vault/OAuth | First scope-reduction attempt; still kept HNSW in core; still bundled QUIC server + Cedar auth. |
| `/Users/justin/Development/AgentAether/aether-db/` | Second, library-only fork (7 crates, ~61 KLOC) | **Most refined extension model in the lineage.** Procedure-pack architecture: JSON manifest, typestate-sealed activation, atomic graph-commit + post-commit audit, native `fn(&Value) -> ProcedureExecutionResult` adapters with JSON Schema gates. SALVAGE.md captures the curation policy. |

Detailed research notes are saved as repo memory (`project_lessons_from_archaeology.md`).

## Templates

Floor-level CI/license/lint scaffolding lives at `/Users/justin/Development/AgentAether/_templates/`. We adopt:

- `ci/rust-baseline.yml` — fmt / clippy `-D warnings` / test / cargo-deny / cargo-audit / file-size cap / no-secret scan
- `ci/deny.toml` — rustls-only, MIT/Apache-2.0 + permissive transitive licenses, crates.io-only sources
- `ci/rust-toolchain.toml` — stable + rustfmt + clippy
- `ci/rustfmt.toml` — edition 2024, max_width 100
- `ci/scripts/check-file-size.sh` — 700 LOC cap
- `ci/scripts/check-no-secrets.sh` — baseline secret-pattern grep

These are the floor; per-repo additions tighten but don't weaken.

## Open design questions

(All v1.0 architectural questions resolved 2026-05-07 — see Decision log D1–D8.)

## Build & test

To be filled in once workspace structure is decided. The floor-level commands once scaffolded:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --all-features
cargo deny check bans licenses sources
cargo audit
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
```

## Conventions

- **Commit format:** conventional commits. `feat(scope):`, `fix(scope):`, `refactor(scope):`. Scope matches the crate or component.
- **Branch model:** trunk + feature branches (TBD; likely `main`-only with feature/* PRs per aether-db).
- **Tests with code:** every new code path lands with a unit test in the same PR. Adversarial tests for parsers/validators/gating logic.
- **Decisions over guesses:** when blocked, surface the question with options and a recommendation. Don't pre-commit.

## Working with the user

- The user is the sole author/owner of all donor codebases (no third-party license entanglements).
- The user prefers **questions presented one at a time** with discrete options and Claude's recommendation, so they can give concise input.
- Marathon mindset: prefer the most robust/performant/non-tech-debt path even when it touches many areas.
- This repo will stay private on GitHub for the foreseeable future.

## Underscore-folder convention (working documents, not committed)

Top-level directories beginning with `_` (e.g., `_spec/`, `_briefs/`, `_review/`, `_design/`, `_plan/`) are **local-only working documents** and excluded from git via `.gitignore`'s `_*/` pattern.

Canonical durable state for design decisions, milestones, work items, and prior art lives in the **MCP graph** (selenedb instance — project node 364, label `selene-db`). The underscore-prefixed folders are dense markdown views of that state plus working scratchpads (briefs to agents, review reports, draft specs).

Implications:
- Tracked files (Cargo.toml, source under `crates/`, CLAUDE.md, LICENSE-*, NOTICE, THIRDPARTY.md, deny.toml, CI workflows, etc.) ship in git.
- Untracked working documents (`_spec/*`, `_briefs/*`, `_review/*`, `_design/*`, `_plan/*`) live on the local filesystem and are reproducible from the graph.
- When briefing an agent (Codex, etc.), specs are referenced via absolute filesystem paths, NOT relative-to-checkout paths, because a fresh `git clone` doesn't include them.
- The MCP graph is the source of truth. If a working document drifts from the graph, the graph wins; regenerate the working document.

## Decision log

### D1 — v1.0 identity: embeddable library only (2026-05-07)

`selene-db` ships at v1.0 as a multi-crate Cargo workspace exposing a Rust library API. **No server, no transport, no auth, no MCP** in this repo.

- ISO GQL is the API surface; consumers (initially the user's `aether-*` repos) embed it and own their own transports / auth / wire format.
- Mirrors the proven `aether-db` library-only shape.
- Rationale: the GQL spec is silent on wire format (clause 4.2.3); the original SeleneDB's ~45 KLOC server crate was the source of 7 critical authz bugs; library-only stays compliance-pure and minimizes attack surface. Library → reference server is straightforward later; the reverse is rewriting.
- Implication: extension system is the *only* surface for adding capabilities. The procedure-pack pattern from `aether-db` is the working model and will inform decision D3.

### D2 — Conformance scope: market-parity (~30–40 features) (2026-05-07)

v1.0 claims conformance with **minimum conformance + ~30–40 commonly-expected optional features**. Concretely the v1.0 claim list includes (subject to refinement during planner/executor work):

- **Updatable graphs**: GD01 (implies GT01 explicit transactions); INSERT/SET/REMOVE/DELETE statements (clause 13).
- **Catalog**: GC04 graph management (`CREATE/DROP GRAPH`); GG02 closed graph types (`CREATE/DROP GRAPH TYPE`); GC03 only for graph-type `IF [NOT] EXISTS` syntax.
- **Procedures**: GP04 (named procedure calls — the extension hook), GP05–GP13 (procedure-local value/binding-table/graph variable definitions), GP14/GP15 (binding-table/graph procedure args).
- **Numeric types**: sized integer variants through 128-bit (GV01–GV14 plus SMALL/BIG synonyms GV05/GV10/GV18/GV19), DECIMAL (GV17), FLOAT32/FLOAT64 (GV21/GV24), and IEEE 754 operation behavior (GA01). GV15/GV16 256-bit integers and GV20/GV25/GV26 non-v1.0 float widths are not claimed.
- **Strings/bytes**: BYTES/BINARY/VARBINARY (GV35), nullability syntax NOT NULL (GV90).
- **Composite types**: LIST (GV50), PATH (GV55), RECORD open + closed (GV45–GV48), reference types GRAPH/NODE/EDGE/TABLE (GV60, GV61).
- **Temporal types**: ZONED DATETIME, LOCAL DATETIME, DATE, ZONED TIME, LOCAL TIME, DURATION (GV39–GV41).
- **Query surface**: composite queries / UNION (GQ03), GROUP BY (GQ15), CASE expressions, advanced path modes — SHORTEST/ALL SHORTEST/ANY (G015–G020), advanced predicates (G110–G115: IS DIRECTED, IS LABELED, IS SOURCE/DESTINATION, ALL_DIFFERENT, SAME, PROPERTY_EXISTS), plus NORMALIZED under the character-string predicate surface.
- **Schema**: GG01 + GG02 (deferred to D3 below for the open-vs-closed-vs-both call), with GG20/GG21 explicit element type names and key label sets.

Rationale: the user chose this over the recommended embedded-ready (~15-feature) target after weighing the trade-off. Marathon mindset accepts the larger scope; aether-db has already brought ~18 KLOC of donor parser/planner/AST forward, which de-risks the language-surface portion of this scope. The runtime/execute layer is the long pole.

Architectural implications:
- The parser implements the supported feature surface; constructs outside that surface raise structured diagnostics with feature IDs at parse time. This **is** the GQL Flagger (clause 24.6). The canonical feature surface is `crates/selene-core/src/feature_register.rs::SUPPORTED_FEATURES`; spec prose and parser checks are generated or verified against it.
- Per-crate LOC budgets revised: GQL crate budget moves from ~25K to ~35–45K to accommodate market-parity scope. Hard cap stays soft above 50K.
- Decision D2 sets the *language* scope. The *runtime / executor* scope is a separate sizing question (factorized vs row-at-a-time; WCO vs hash-join) — captured in a future decision after extension API and persistence are settled.

### D3 — Schema model: both GG01 + GG02 (2026-05-07)

v1.0 supports **both** open graphs (GG01, schemaless) and closed graphs (GG02, declared graph type). Per-graph choice at `CREATE GRAPH` time:

- **Open graph** — accepts any well-formed nodes/edges; no commit-time validation against a graph type.
- **Closed graph** — requires a `<graph type>` declaration; commits validate node-type / edge-type / property-type / label-set conformance per clause 18 + 4.16.
- The `aether-*` schema-packs-ship-with-code convention will use closed graphs; ad-hoc embedded users can use open graphs.

Implications:
- Parser must support `CREATE GRAPH <name> :: <graph type>` and `CREATE GRAPH TYPE` DDL (clause 12).
- Graph type expressions, node/edge type elements, property/value/field types (clause 18.1–18.13) all in scope.
- A type-validator runs at commit for closed graphs; bypassed for open graphs. The validator is a separate module and feature-gated by GG02 in the claimed-features set.
- Catalog stores graph-type definitions; `DROP GRAPH TYPE` requires no extant closed graphs of that type.
- Tests must cover: GG01-only graph mutations, GG02 conformance failures, cross-graph queries that mix open and closed graphs (allowed).

### D5 amendment — selene-vector design specifics (2026-05-07, post second-research-pass)

The second-pass vector-index research revealed several architectural points that refine D5 without changing the core decision. Captured here as an amendment rather than rewriting D5:

- **Adopt NaviX-style filter-aware traversal from day one.** `NaviX: A Native Vector Index Design for Graph DBMSs` (Sehgal et al., VLDB 2025) is the exact selene-vector use case — pre-filtering with adaptive heuristics that pick strategy per-iteration based on local selectivity. Highest-priority paper to read. Filter-aware traversal must use selene-graph's scalar indexes (RoaringBitmap label index + TypedIndex) to feed candidate sets. ACORN-style predicate-subgraph traversal as a fallback for high-cardinality predicates.
- **Adaptive ef from day one.** Static-ef HNSW is the documented pain point in 2025 papers. Per-query target-recall API; retrofitting later is harder than designing for it.
- **SPFresh / LIRE-style local rebalance for incremental updates.** Tombstone-based soft delete + periodic local rebuild. Document that >10–15% deletion ratio triggers rebuild. Tombstones skipped during traversal, not just at result time. Avoid the "global rebuild on N% churn" pattern.
- **Mine `hnsw_rs` (MIT/Apache-2.0) for HNSW reference code.** Do NOT wrap as a hard dependency. The implementation is dual-permissive — code can be mined under license-compatible terms. Implement filter-aware extensions on top.
- **`simsimd` (Apache-2.0) for SIMD distance kernels.** 350+ kernels across AVX2/AVX-512, NEON, SVE/SVE2, RISC-V RVV. f32, f16, bf16, i8, **bit vectors** (critical for binary quantization). Avoid hand-rolled SIMD in v1.0; re-evaluate after profiling.
- **Rename / replace "PolarQuant".** The donor's polar-coord quantization scheme has a critical name collision: three 2025–2026 LLM-quantization papers use "PolarQuant" for KV-cache and weight quantization (Wu et al. NeurIPS 2025; Han et al. AISTATS 2026; Vicentino 2026). Selene-vector must NOT publicly market quantization as "PolarQuant." For v1.0: ship scalar 8-bit + binary quantization tiers (with reranking) and **evaluate RaBitQ / SAQ** (Hugging Face #2509.12086) as the principled successor to PQ — both academically published with theoretical error bounds, MIT/A2-friendly. The polar-coord scheme can be deferred to v1.x under a new name (e.g., `SelenePolar`) only if it benchmarks competitive with RaBitQ/SAQ.
- **No embedding model bundling.** Confirmed by both donor lesson (v1.2 deleted EmbeddingGemma, ~4 KLOC) and 2025–2026 ecosystem signals (Voyage / Cohere / OpenAI all ship binary + Matryoshka — embedding production is a fast-moving caller concern). Selene-vector accepts pre-computed `Arc<[f32]>` only.
- **microsoft/DiskANN main branch is now 99.9% Rust under MIT.** Includes FreshDiskANN + FilterDiskANN. Directly mineable for v1.x disk-based extension; out of v1.0 scope (in-memory only).

### D9 — Technology stack and crate-level library choices (2026-05-07)

The second-pass library survey (4 deep-research agents) verified license, maintenance, and adoption for every dependency category. The following decisions consolidate the pass into a binding crate-level choice list. Rationale per crate is in `_spec/09-engineering-discipline.md` (to-be-authored); the decisions themselves are bound here.

**Workspace dependencies** (versions pinned at decision date 2026-05-07; bumps tracked per-PR):

| Purpose | Crate | License | Rationale |
|---|---|---|---|
| Persistent CoW collections | `imbl 7.0` | MPL-2.0 | D7 already; only mature complete persistent-collection family in Rust. |
| Cheaper Arc for ArcSwap payloads | `triomphe 0.1` | MIT/A2 | Skips weak-count overhead vs `std::sync::Arc`. |
| Atomic snapshot publication | `arc-swap 1` | A2 | Sub-nanosecond reads; donor-proven. |
| Write coordination | `parking_lot 0.12` | MIT/A2 | Donor-proven; faster than std mutexes. |
| Lock-free read-mostly maps | `papaya 0.2` | MIT | Specifically the procedure-pack registry hot path. |
| Bitmap label indexes | `roaring 0.11` | A2/MIT | Donor-proven; ecosystem standard. |
| FxHashMap for in-process maps | `rustc-hash 2` | MIT/A2 | Faster than default SipHash for non-DOS-exposed in-process. |
| String interning | `lasso 0.7` | MIT/A2 | Donor-proven; small surface; watchlist for maintenance. |
| SSO short strings | `smol_str 0.3` | MIT/A2 | Property values; avoids allocation for ≤23-byte strings. |
| Parser | `pest 2.8` | MIT/A2 | Donor's grammar is irreplaceable; switching to winnow/chumsky is rewrite-risk. |
| WAL serialization | `postcard 1.1` | MIT/A2 | Stable wire format; sequential write-once. |
| Snapshot serialization | `rkyv 0.8` | MIT | **Zero-copy mmap** for snapshot replay; orders of magnitude faster cold start vs postcard. |
| Compression | `zstd 0.13` | MIT | Best ratio; competitive at low levels. |
| Cryptographic hash (WAL/snapshot integrity) | `blake3 1.8` | CC0/A2/A2-LLVM | **4–10× faster than sha2** single-threaded; permissive triple license. |
| SHA-256 (compatibility paths only) | `sha2 0.11` | MIT/A2 | Kept for any external compatibility surface that demands SHA-256. |
| UUID v4/v7 | `uuid 1.23` | MIT/A2 | UUIDv7 is now stable in `uuid` 1.x; no `uuid_unstable` cfg flag needed. |
| Decimal type (GV17) | `rust_decimal 1.42` | MIT | Fixed-precision 28-digit; matches GQL DECIMAL semantics. |
| Temporal types (GV39–41) | `jiff 0.2` | Unlicense/MIT | Only Rust temporal lib with semantically-correct DST-aware DURATION arithmetic. Pin minor; plan 1.0 upgrade. |
| JSON Schema validation | `jsonschema 0.46` | MIT | Drafts 4/6/7/2019-09/2020-12; production-quality. |
| JSON Schema generation | `schemars 1.2` | MIT | Generate procedure-pack manifest schemas from Rust types; eliminates schema/validator drift. |
| JSON | `serde_json 1` | MIT/A2 | Default; manifest parsing is not perf-critical. |
| Tracing/logging | `tracing 0.1` + `tracing-subscriber 0.3` | MIT | De facto. Embedders bridge to OTel. |
| Metrics | `metrics 0.24` | MIT | Vendor-neutral histogram/counter facade. |
| High-resolution timing | `quanta 0.12` | MIT | Sub-microsecond intra-process timing in hot paths and benchmarks. |
| Library error idiom | `thiserror 2` | MIT/A2 | Structural error variants; maps to GQLSTATUS codes. |
| Diagnostic rendering | `miette 7.6` | A2 | Source-pointer rendering with `Diagnostic` derive over thiserror. |
| Property tests | `proptest 1.11` | MIT/A2 | Strategy-based; rich shrinking; default for randomised tests. |
| Structured fuzzer input | `arbitrary 1.4` | MIT/A2 | For `cargo-fuzz` parser fuzzing. |
| Bench (end-to-end) | `criterion 0.8` | MIT/A2 | De facto. |
| Bench (instruction-precise CI gate) | `iai-callgrind 0.16` | MIT/A2 | Deterministic instruction counts survive noisy runners. |
| Parametrised tests | `rstest 0.26` | MIT/A2 | Replaces hand-rolled test-matrix loops. |
| Snapshot tests | `insta 1.47` | A2 | Locks AST/plan/diagnostic output stability. |
| SIMD distance kernels (selene-vector) | `simsimd` | A2 | 350+ kernels; bit-vector support for binary quantization. |
| Mine, do NOT depend on | `hnsw_rs` | MIT/A2 | HNSW reference code for selene-vector. |

**Architectural splits worth noting:**

1. **WAL = postcard, Snapshot = rkyv.** Different access patterns: WAL is sequential write-once read-once (postcard's strength); snapshot is cold-start mmap-then-validate (rkyv's strength). This split avoids the temptation to make one format do both jobs poorly.
2. **Hashing = blake3.** Single change from earlier WAL/snapshot reasoning (D6 said sha2). blake3 is 4–10× faster on the WAL hot path and snapshot finalisation; license is dual-permissive.
3. **`papaya` for the procedure-pack registry.** Read-mostly lookup of `pack_id → Arc<LoadedPack>`; lock-free reads avoid the global RwLock during invocation.

**Crates explicitly NOT adopted, with reason:**

- `tokio` in core (too heavy / viral for an embeddable library; sync engine, embedders bridge with `spawn_blocking`).
- `cht` (AGPL-3.0-or-later, license-incompatible).
- `dashmap` (deadlock-on-iter-while-ref hazard; `papaya` is the safer modern equivalent).
- `winnow` / `chumsky` (great parsers, but pest + miette covers our case without the donor-grammar rewrite cost).
- `bincode 3` / `bitcode` / `flatbuffers` / `capnp` (postcard + rkyv covers WAL + snapshot).
- `xxhash-rust` (BSL-1.0, license-incompatible).
- `valico` (stale, draft 4 only).
- `snap` (stale, BSD-3, lz4_flex covers if needed).
- `chrono` for ZONED DATETIME (DST arithmetic story is wrong; jiff is correct).

**Engine-architecture findings worth noting (informs future specs):**

- **Free Join / COLT** (Wang et al. SIGMOD 2024) — column-oriented lazy trie that unifies hash-based binary joins with WCO multi-way intersections. Public Rust reference implementation. Sidesteps donor's d! sorted-index pain. Highest-leverage planner change. Will be settled in `_spec/08-iso-gql-planner-and-executor.md`.
- **WanderJoin sampling cardinality** (Hu et al. ACM TODS 2024) — pragmatic upgrade over donor's static `EdgeStatistics`. No ML, small code footprint. Will be settled in spec 08.
- **FESIA / HERO SIMD set intersection** — 2–5× on the WCO inner loop. Use `simsimd` or `wide` + `multiversion` for the intersection kernels.
- **opengql/grammar** (LDBC ANTLR4 reference, Apache-2.0) — selene-db's pest grammar should mirror its rule names; cross-validate against ANTLR-derived test snippets. Will be settled in spec 07.
- **openCypher TCK + openCypher Cucumber/Gherkin scenarios** — many translatable to GQL = thousands of free regression tests. Will be settled in spec 10.
- **LDBC SNB Interactive v2 + BI** as CI perf gates at SF1/10. Will be settled in spec 10.
- **ISO/IEC 39075:2024/CD Cor 1 in flight** (registered 2026-01-05) — track for parser updates once published.
- **KuzuDB acquired by Apple Oct 2025; archived publicly** — reference design preserved, no longer evolving in public.
- **GraphLite (Rust, Sled, Nov 2025)** — selene-db's closest direct comparable; worth studying their grammar mapping and conformance approach.

### D10 — Transaction isolation: single graph write-lock + MVCC reads (2026-05-07)

Writers acquire the per-graph write-lock (`parking_lot::RwLock` write guard) at `START TRANSACTION` and hold it across the entire read+write phase until `COMMIT` or `ROLLBACK`. Readers run lock-free against the `ArcSwap`-published immutable snapshot at any time, including while a writer holds the lock. The published snapshot only updates at successful `COMMIT`. This is the strict-serializable contract per ISO clause 4.6 and `IE002`/`IE004`: only one transaction is effectively active for write purposes at a time.

Rationale: simplest correct implementation of declared serializable; no read-set tracking, no validation, no retry loop. Concurrency is the caller's responsibility and the caller is expected to serialize its own writes anyway because v1.0 is library-only (D1). Runtime details live in `_spec/03-property-graph-and-concurrency.md` §6.

### D11 — ID allocation: real `NodeId` / `EdgeId` allocated under the write-lock; aborted-tx IDs become permanent holes (2026-05-07)

A per-graph `IdAllocator` holds the next-`NodeId` and next-`EdgeId` counters. While a transaction holds the write-lock (per D10), it freely increments the counters; the increments are visible only to that transaction's `Mutator` until commit. On commit, the counters' new values are published as part of the `ArcSwap` swap. On abort, the counters are NOT rolled back: the IDs that were allocated become permanent holes in the dense monotonic sequence, identical in effect to a node that was created and then dropped.

Spec 02 §4's "holes from `DROP NODE` are not reused" rule generalizes naturally: aborted-transaction holes obey the same invariant. No free-list, no temp-ID rewrite, no commit-time renumbering. Read-your-writes within an aborting transaction is preserved because IDs are already real-shaped. The identity rule lives in `_spec/02-data-model.md` §4; lifecycle details live in `_spec/03-property-graph-and-concurrency.md` §6.

### D12 — Audit: opaque caller-supplied principal byte-slot in the WAL header (2026-05-07)

Every WAL entry header carries `principal: Option<Arc<[u8]>>`. The bytes are caller-defined and never parsed, validated, or interpreted by selene-db. The caller (for example, an `aether-*` server) supplies them at commit time via `Mutator::commit_with_principal(bytes)`; the convenience `commit()` is equivalent to `commit_with_principal(None)`. `selene-persist` round-trips the slot through WAL serialization, snapshot capture, and recovery replay. Audit replay is `selene_persist::wal_iterate(filter)`.

Audit-outlives-subjects is satisfied because the WAL is append-only: a deleted node's mutation history remains queryable from prior WAL entries indefinitely. This honors D1 (selene-db never owns auth/principal modeling) and the Poseidon discipline-stack (audit records do not couple to subject lifecycle). Cap on principal slot: 4096 bytes per entry, rejected with `GQLSTATUS 22023`. Rationale for 4 KiB: comfortably exceeds Cedar entity-uid + signature payloads observed in Aether; small enough to keep WAL entry sizes predictable. The durability format lives in `_spec/04-persistence-format.md` §3.

### D4 — Extension architecture: procedure-pack + feature-gated workspace modules (2026-05-07)

The extension system is the procedure-pack model from `aether-db`, ported with the per-tier-Context fix. Concretely:

- **One trait** `Procedure` with associated `Context` type. Each implementation registers via `OnceLock` static registration at process start (the pattern that worked in donor's RdfProvider/VectorProvider).
- **Per-tier `Context` types** — graph-tier procedures get `&Graph`; mutation-tier gets `&Graph + &Mutator`; persist-tier gets `&Graph + &Persist`. Prevents the donor's `Procedure::execute(...hot_tier: Option<&HotTier>...)` regret where every procedure carried a TS reference it didn't use.
- **JSON manifest** (`procedure-pack.json`) per pack, declaring procedures, mutability, capability_required, idempotency, limits, cost_model. JSON-only (no TOML).
- **JSON Schema gates** on every `execute` call — input validated against the procedure's input schema, output validated before returning.
- **Typestate-sealed activation** — `SealedPackActivation` is constructible only via a `seal_for_activation()` step that recomputes pack hash and asserts generated artifacts byte-identical. State machine: `uploaded → validating → {rejected | staged} → active → {deprecated, disabled}`.
- **Atomic graph-commit + post-commit audit** — registry state, `active_generation` counter, and audit ledger row commit in one `graph.mutate()` block; audit emission happens after the graph commit (ordering rule: "audit lag is recoverable; fictional audit is not").
- **Invocation via the normative ISO GQL `CALL` clause** — no new grammar; uses spec hook IW010.
- **Compile-time linking, no runtime plugin loading.** WASM is not in v1.0 scope; the `Procedure` trait shape is compatible with adding a WASM impl later if the need emerges.
- **Vectors / time-series / RDF / GraphRAG ship as separate workspace crates gated by Cargo features.** Default features = none (lean by default); consumers opt into the capabilities they need.
- **Generated MCP tool surface** is *not* in selene-db itself — that's a downstream consumer concern (the user's `aether-*` repos generate it). selene-db produces the registry data; consumers project it to MCP/HTTP/whatever.

The procedure-pack registry is itself stored as platform-owned graph nodes inside the live graph (labels: `ProcedurePack`, `ProcedurePackVersion`, `ProcedurePackActivation`; edges: `HAS_VERSION`, `HAS_ACTIVATION`). Eats its own dog food and means activation lifecycle is queryable via standard GQL.

Implementation order: trait + Context, then registry, then activation state machine, then JSON manifest validator, then the normative `CALL` planner integration. JSON Schema validation crate to be picked from a small short-list (`jsonschema`-rs is the donor's choice; pick a dep with a strong adoption signal).

### D5 — Extension-owned vector values and HNSW packaging (2026-05-07)

- **Superseded by the amendment below:** the early compromise put a typed vector primitive in `selene-core` for postcard layout stability. BRIEF-02 replaces that with the permanent opaque `Value::Extended` variant so extension-owned value types never become core enum members.
- **HNSW index implementation lives in an extension crate `selene-vector`.** `selene-graph` does NOT contain HNSW code at all. HNSW is physically excluded from the binary when `vector` feature is off.
- **`selene-graph` exposes one `IndexProvider` trait** — a carefully-scoped addition to D4's procedure-pack model — letting extension crates register custom indexes. The trait surface includes:
  - Mutation event hook: `on_node_change(NodeId, ChangeKind, &PropertyMap)` etc.
  - Snapshot hook: `serialize_to(SectionWriter) / restore_from(SectionReader)` so the index data lands in graph snapshots as a tagged section.
  - WAL hook: optional; if the index needs WAL participation (e.g., for incremental persistence between snapshots) it provides a `replay(&[Change])` method.
  - Lifecycle: indexes are registered at graph construction time before any mutations.
- **`selene-vector` ships procedures**: `vector.search`, `vector.cosine`, `vector.quantize`, etc. via the procedure-pack model. The procedures consume the registered HNSW index through their per-tier `Context`.
- **Implication for `selene-graph` budget:** the index hook keeps selene-graph graph-focused. Graph crate stays in its ~8K LOC budget; vector code is wholly external.

This is the model that operationalizes the user's stated vision ("move vectors to extensions") concretely: vector *typing and operations* stay extension-owned, while core carries only the stable opaque extension payload variant.

#### D5 amendment — 2026-05-07 — Vector pushed fully to extension (BRIEF-02 / F-007 / marathon-directive)

The former typed vector enum variant is removed from core. `Value::Extended { type_id, payload }` is the canonical mechanism for any non-mandatory-spec value type. `selene-vector` reserves `ExtensionTypeId(0x00000100)`, registers a `ValueTypeAdapter`, and is the only place vectors exist as a typed concept. Postcard layout stability is preserved by `Extended` being the permanent enum variant.

Rationale: this is the marathon path. It eliminates the extension-leak risk from REVIEW-01 F-007, aligns with the project's "move vectors to extensions" vision, and leaves no future enum churn for other extension-owned value types such as geometry or specialized decimal forms.

### D6 — Persistence: WAL+snapshot from day one in `selene-persist` (2026-05-07)

`selene-db` is durable from v1.0 via a `selene-persist` crate that owns the WAL and snapshot formats but **does not own the graph** — the caller decides when/whether to persist.

Format choices:
- **WAL** — magic `SLDB` (or similar selene-db-specific four-byte tag), version 1, 16-byte header, entry header with sha2-256-truncated checksum (low 32 bits) + sequence (u64) + HLC NTP64 timestamp + origin byte (Local / Replicated). Payload: `zstd(postcard(Vec<Change>))`, compressed when ≥128 bytes. Max payload 256 MiB.
- **Snapshot** — magic `SLSN` (or similar), version 1, **TLV-tagged sections** (improvement over donor's positional sections). Sections include: Metadata, Nodes, Edges, Schemas, plus an extension-section registry that lets `IndexProvider` impls (e.g., HNSW from D5) declare and own their own sections without bumping the snapshot format version. Atomic write via tmp+rename. sha2-256-truncated 128-bit hash for integrity.
- **Recovery** — `find_latest_snapshot → read_snapshot → take extension sections → replay WAL entries past snapshot_seq with upsert semantics → return RecoveryResult { nodes, edges, next_node_id, next_edge_id, sequence, schemas, extension_sections }`.

Key implementation rules carried forward from donor archaeology:
- **Every write surface routes through the persist funnel.** Schema mutations, procedure-pack activations, index updates — all go through `Mutator → WAL → snapshot eligibility`. No bypass paths. Type-enforced where possible.
- **Persist crate has zero knowledge of `Graph`.** Takes a `&[Change]` and produces durability; reads return raw `RecoveryResult` for the caller to apply.
- **Format-version migration story:** snapshot v1 uses TLV sections so adding a section is non-breaking. WAL v1 will get v2 only if the entry header itself needs to change. Magic+version envelopes always.
- **HLC timestamps and origin byte from day one** so federation / replication can land later without WAL format churn.

`selene-persist` LOC budget: ~5 KLOC, donor-equivalent. Pluggable storage backends (RocksDB / Sled / cloud) are explicitly **not** v1.0 scope; if a real second backend emerges, abstract as a `Storage` trait in a focused refactor at that time.

### D7 — Concurrency model: ArcSwap + RwLock + imbl CoW collections (2026-05-07)

The graph uses a three-layer concurrency model:

- **Reads:** `ArcSwap<Arc<SeleneGraph>>` — readers do `arc_swap.load()` for sub-nanosecond, lock-free access to a consistent snapshot of the graph.
- **Writes:** `RwLock<SeleneGraph>` — single-writer at any instant. Mutations accumulate in a `Mutator` (transaction); on commit, the writer clones the affected sub-structures (cheap due to imbl structural sharing), applies the diff, and `arc_swap.store()`s the new `Arc<SeleneGraph>`.
- **Storage primitives:** `imbl` — `imbl::HashMap`, `imbl::Vector`, `imbl::OrdMap`. Structural-sharing CoW means commit-time clone is `O(log_64 n)` per modified collection, not `O(n)`. Million-node graphs commit in microseconds, not milliseconds.

**License posture:** `imbl` is MPL-2.0, which is **file-level copyleft, not project-level**. selene-db's dual MIT-OR-Apache-2.0 license is unaffected; only imbl's own files carry MPL terms downstream. Selene-db's `deny.toml` adds `"MPL-2.0"` to the `[licenses].allow` list with an explanatory comment — a one-line deviation from the AgentAether template's strict permissive posture, justified by imbl being the only mature complete CoW collection family in the Rust ecosystem (verified 2026-05-07; `imbl` 222K downloads/month, used in 343 dependents).

**Why not the alternatives:**
- `immutable-chunkmap` (MIT/Apache) only covers BTree-style sorted maps; no HashMap or Vector — would force hand-rolling those.
- Hand-rolling HAMT + RRB-tree under MIT/Apache is ~3–6 weeks of subtle work; HAMT's archived author noted "with Rust's ownership semantics, persistent data structures are needed less often" — a real hint that this work would compete with the work that actually defines selene-db.
- Donor's full-clone-on-FxHashMap pattern is the user-flagged "fallback" — accepted as tech debt the user explicitly does not want.

**Implications for design:**
- `selene-graph` storage primitives default to `imbl::HashMap` / `imbl::Vector` for hot adjacency / property data; `roaring::RoaringBitmap` for label index bitmaps (already MIT).
- Single-writer simplifies the mutation funnel — every write goes through `Mutator → Commit → ArcSwap::store` with no concurrent-writer reasoning needed.
- The `Mutator` is the typed funnel point (per the donor archaeology lesson "every new I/O surface routes through ONE mutation/auth funnel — enforced by types").
- MVCC and multi-writer optimization are explicitly out of v1.0 scope; revisit only if a concrete benchmark demands it.

### D8 — Workspace shape: multi-crate, no umbrella (2026-05-07)

selene-db is a Cargo workspace with multiple focused crates and **no `selene` umbrella facade crate**. Consumers depend on the sub-crates they need by path or version dep.

**v1.0 mandatory crates** (at process start, every consumer pulls these):
- `selene-core` — foundation types (Node, Edge, Value with all 4 mandatory spec types + `Value::Extended`, IStr, PropertyMap, LabelSet, schema types, Codec trait, Origin enum, Changeset). Zero deps on other selene crates. LOC budget ~3K.
- `selene-graph` — in-memory property graph. ArcSwap+RwLock+imbl model. RoaringBitmap label indexes, TypedIndex, IndexProvider trait (the extension hook for HNSW etc.). LOC budget ~8K.
- `selene-gql` — ISO GQL parser (pest), AST, planner with rule-based optimizer + WCO joins, pattern executor, columnar runtime, mutation builder. LOC budget ~35–45K (D2 market-parity scope).
- `selene-persist` — WAL (`SLDB` magic) + snapshot (`SLSN` with TLV-tagged sections) + recovery. Does not own the graph. LOC budget ~5K.
- `selene-pack` — procedure-pack registry, validator, activation state machine, native procedure runtime. The extension surface per D4. LOC budget ~10K.
- `selene-algorithms` — graph algorithms (PageRank, WCC, SCC, Dijkstra, etc.). Independent of GQL runtime. LOC budget ~5K.
- `selene-testing` — test fixtures (synthetic graph generators, assertion helpers). Internal use only. LOC budget ~2K.

**v1.0 opt-in extension crates** (depend on the mandatory crates, register procedures + indexes):
- `selene-vector` — HNSW index (registered via D5's IndexProvider hook) + vector procedures (search, cosine, quantize). `selene-polar-quant` is an internal placeholder for the donor polar-coordinate quantization experiment; final public name resolved during selene-vector M8 implementation. LOC budget ~6K.
- (Future, post-v1.0:) `selene-timeseries`, `selene-rdf`, `selene-graphrag`, `selene-fulltext`. Each ships as a separate workspace crate gated by Cargo features at the consumer level.

**No umbrella** means: refactoring crate boundaries doesn't break the public surface; each crate has its own `Cargo.toml` version and changelog entry; per-crate dependency rules are visible (selene-graph cannot accidentally pull a vector dep). Mirrors aether-db's library-only shape.

**Crate-level dependency rules** (enforced by code review and `cargo-deny`):
- `selene-core` depends on no other selene crate.
- `selene-graph` depends only on `selene-core`.
- `selene-gql` depends on `selene-core` and `selene-graph`.
- `selene-persist` depends only on `selene-core` (takes `&[Change]`, returns RecoveryResult; never sees `Graph`).
- `selene-pack` depends on `selene-core`, `selene-graph`, `selene-gql` (the procedure runtime hooks into GQL's CALL planner).
- `selene-algorithms` depends only on `selene-core` and `selene-graph`.
- `selene-testing` depends on the mandatory crates as a test-only consumer.
- Extension crates (`selene-vector` etc.) depend on the mandatory crates plus the procedure-pack/index hooks.
