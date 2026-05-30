# AGENTS.md — selene-db

Canonical startup context for AI coding agents working in this repository. Follows the [agents.md](https://agents.md) standard. Read by both Codex CLI (native `AGENTS.md` discovery) and Claude Code (via the `CLAUDE.md` symlink in this directory). When the two differ, this file is the source of truth.

This file covers **what is specific to selene-db**: architecture, hard rules, decisions, build commands. The universal partner-workflow contract (Briefback gate, Stage-0→3 cadence, PR shape, fix-cycle protocol) lives at the user level in `~/.codex/AGENTS.md` (executor-side) and `/Users/justin/Development/_agent_helpers/partner-workflow.md` (full 573-LOC lead-oriented spec). Read those once per session if you haven't; do not duplicate their content here.

The drift-prone status surface (current milestone, last-merged BRIEF, in-flight PR) is intentionally **not** in this file. Authoritative state lives in the MCP `selenedb` graph (project node 364) and in `_design/milestone-log.md`; recent history is `git log`.

---

## Mission

`selene-db` is a strict ISO/IEC 39075:2024 GQL property graph engine, designed greenfield to keep the core small and high-performance and to move every non-graph capability (time-series, vectors, RDF, GraphRAG, etc.) into a clean extension/module system.

This is a **marathon, not a sprint**. No shortcuts. Every decision optimizes for correctness, performance, and a stable extension contract — even when that means more work today.

v1.0.0 shipped 2026-05-16 (tag `v1.0.0`). Post-1.0 work is tracked under explicit release-scope documents in `_design/`.

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

Error codes are GQLSTATUS (§23.1 Table 8), **never** SQLSTATE. Mutation/graph-type-violation prefix is `G1xxx` / `G2xxx`. Predicates live in §19. `LIKE`, `BETWEEN`, `^`, `XX500`, `0A000`, `42883`, `22023` are SQL drift — reject on sight.

## Hard rules

1. **Strict ISO GQL compliance.** Anything not in the spec lives in extensions or impl-defined hooks. Never extend the GQL grammar itself.
2. **No shortcuts.** No "phase X scaffold," no placeholder framing, no TODO-it-later code. Documents describe the layer the type ships, not the order of sprints.
3. **`#![forbid(unsafe_code)]` workspace-wide.** Bounded RFC-specified surfaces may be exceptions, with justification in `// Why:` comments.
4. **`missing_docs = "deny"` workspace-wide.** Every `pub` item carries a rustdoc with intent, invariants, or non-obvious behavior. Dead surfaces get demoted to `pub(crate)`, not stubbed.
5. **File-size cap: 700 LOC.** Refactor or split before approaching the cap. Enforced by `.github/scripts/check-file-size.sh`. **This is the only LOC gate.**
6. **Per-crate LOC guidance (NOT merge-gating):** rough budgets are `selene-core` ~3K; `selene-graph` ~8K; `selene-gql` ~35–45K; `selene-persist` ~5K; `selene-pack` ~10K; `selene-algorithms` ~5K; `selene-testing` ~2K. Structural hints for *when to split into a new module/extension crate*; not acceptance bars.
7. **rustls-only TLS posture.** No native-tls, no openssl-sys. Enforced in `deny.toml`.
8. **Dual MIT OR Apache-2.0 license.** Permissive transitive license allow-list per `deny.toml` (with one explicit `MPL-2.0` exception for `imbl`, see D7).
9. **Latest stable Rust.** Pinned in `rust-toolchain.toml` (currently 1.95.0). Latest published deps with strong adoption signals.
10. **Never hand-roll crypto / TLS / async runtime / serialization primitives.**
11. **Every new I/O surface routes through a single mutation/auth funnel — enforced by types.** No "this new endpoint forgot to go through the funnel" class of bug.
12. **GQL is the sole query and mutation interface.** No SQL/Cypher/SPARQL grammar in the engine; if needed, ship as extensions that translate to GQL or call procedures.
13. **Attribution.** Donor codebases (SeleneDB, AetherDB, aether-db, rusty-bacnet) are the user's own work and need no per-line attribution. Third-party code (crates, blog posts, vendored snippets) does:
    - `NOTICE` — Apache-2.0-style notice naming third-party copyright holders for bundled/adapted code.
    - `THIRDPARTY.md` — auto-generated from `Cargo.lock` via `cargo-about`; CI-gated against drift. Regen on every version bump or dep change.
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
| `selene-core` | none | Foundation types: `Value` (4 mandatory ISO types + `Value::Extended` + `Value::ExternalString(Arc<str>)` per D20), `IStr`, `PropertyMap`, `LabelSet`, schema types, `Codec`, `Origin`, `Changeset` (includes `SchemaChange::ProcedurePackLifecycle { event: PackLifecycleEvent }` for D18 audit). |
| `selene-graph` | core | In-memory property graph: ArcSwap+RwLock+imbl, RoaringBitmap label index, TypedIndex, `IndexProvider` trait, `Mutator` write funnel, `GraphTypeDef` runtime binding. |
| `selene-persist` | core | WAL (`SLDB` magic) + snapshot (`SLSN` TLV-tagged sections) + recovery. **Never sees `Graph`** — takes `&[Change]`, returns `RecoveryResult`. |
| `selene-gql` | core, graph | ISO GQL parser (pest), AST, semantic analyzer, planner, optimizer, executor, `ProcedureRegistry` trait. |
| `selene-pack` | core, graph, gql, persist | Procedure-pack registry, manifest validator (JSON Schema 2020-12 gates), typestate-sealed activation state machine, atomic mutation-funnel audit (`GraphCommitSink`), canonical blake3 content_hash, 4 platform built-ins. |
| `selene-algorithms` | core, graph | Graph algorithms: `GraphProjection` + `ProjectionCatalog` foundation + 15 public algorithm surfaces (structural / pathfinding / centrality / community) + D21 snapshot harness. Independent of GQL. |
| `selene-testing` | core, graph | Test fixtures, synthetic graph generators, pure-mirror snapshot-harness DSLs (e.g., `pack_corpus` per D21). Consumed via `[dev-dependencies]` only. |

Opt-in extension crates:

| Crate | Depends on | Owns |
|---|---|---|
| `selene-algorithms-pack` | core, graph, gql, pack, algorithms | Procedure-pack surface for graph algorithms (`algo.projection_*`, `algo.pagerank`, `algo.wcc`, `algo.dijkstra`, `algo.louvain`, etc.). |

Future opt-in extension crates: `selene-timeseries`, `selene-rdf`, `selene-graphrag`. **No umbrella crate.** Crate boundaries are enforced by code review and `cargo-deny`.

## Decision log

All v1.0 architecture is settled. D1–D19 are foundational; D20–D21 and D25 record load-bearing structural patterns established after the v1.0 foundation. Fields named in a row reflect the *current* shape after any amendments.

| ID | Decision | Canonical home |
|---|---|---|
| D1 | v1.0 = embeddable library only (no server, no transport, no auth, no MCP) | `_spec/01` §4 |
| D2 | Conformance: minimum + ~30–40 parser-reachable optional features | `_spec/01` §5 |
| D3 | Schema: both GG01 (open) and GG02 (closed); per-graph choice | `_spec/02` §6 |
| D4 | Procedure-pack extension model + feature-gated workspace modules | `_spec/05` §3, §5 |
| D5 | Vector via `Value::Extended`; externalized to a separate dedicated project in the 2026-05-30 consolidation | `_spec/02` §3, `_spec/06` §3 |
| D6 | WAL+snapshot from day one in `selene-persist` | `_spec/04` |
| D7 | Concurrency: `ArcSwap` + `parking_lot::RwLock` + `imbl` CoW; `imbl` MPL-2.0 exception accepted | `_spec/03` §4 |
| D8 | Multi-crate, no umbrella; linear dep direction | this file (Crate map), `_spec/03` §8 |
| D9 | Workspace dependency choices (imbl, rkyv, postcard, blake3, jiff, …) | `_spec/09` §3 |
| D10 | Transaction isolation: single graph write-lock + lock-free reads (strict-serializable) | `_spec/03` §6 |
| D11 | ID allocation: aborted-tx IDs become permanent holes. **Amended (BRIEF-Item-4a / D22):** *external* `NodeId`/`EdgeId` (monotonic `u64`) stay permanent & never-reused (an aborted-tx id is still a permanent hole); the *internal* `RowIndex(u32)` is decoupled from the id (reads resolve via id↔row maps, never `id == row+1`) so 4b/4c compaction can remap it. After recovery an aborted-tx hole id resolves `NotFound` (was `NotAlive`) — a live-vs-recovered consistency fix. | `_spec/02` §4, `_spec/03` §6; BRIEF-Item-4a |
| D12 | Audit: opaque caller-supplied principal byte-slot in WAL header | `_spec/04` §3 |
| D13 | Snapshot section tag: 8-byte composite (provider + sub-tag); 24-byte row | `_spec/04` §4 |
| D14 | Snapshot serialization: `rkyv` archive over per-section `Vec<(K, Row)>`. **Amended (BRIEF-Item-4a / STEP 9):** `CORE/NODE` & `CORE/EDGE` are now **positional** — each row stores its explicit external `NodeId`/`EdgeId` (`*Id::TOMBSTONE` for aborted-tx holes), recovery places rows by section position (not `id-1`), and id-uniqueness-excluding-tombstone replaces the sorted-by-id requirement; `CORE/SCMA`/`CORE/GTYP` stay sorted-unique. `SNAPSHOT_VERSION_MINOR` bumped 0→1 = clean break (pre-STEP-9 snapshots rejected with `UnsupportedVersion`; internal-id remap headers + dual decoder for pre-compaction snapshots deferred to 4c). **Amended (JSON/L1c-d):** `CORE/GTYP` collapsed to a single `GTYP_VERSION = 1` (clean greenfield break — legacy V1/V2/V3 magic decoders removed; pre-existing `0xB6`/`0xB7` GTYP snapshots rejected with `ProviderError`); `PropertyTypeDef` gains `record_field_types: Option<RecordFieldTypes>`. `SNAPSHOT_VERSION_MINOR` is **unchanged** — the GTYP version lives inside the `b"GTYP"` section payload, independent of the SLSN container version. | `_spec/04` §4; BRIEF-Item-4a; JSON/L1c-d |
| D15 | Recovery: three-step (**MANIFEST read** → snapshot apply → WAL replay) — the `SLMF` epoch descriptor is authoritative (names the live snapshot + WAL floor); pre-MANIFEST directories fall back to the legacy two-step path. `RecoveryProvider` lives in `selene-persist`, `IndexProvider` lives in `selene-graph`; both use `&self` receivers | `_spec/06` §3, §12; BRIEF-148 |
| D16 | `ProcedureRegistry` trait owned by `selene-gql`; `lookup` takes `&[IStr]`; embedder injects `&dyn ProcedureRegistry` | `_spec/08` §7 |
| D17 | Procedure tiers: per-tier concrete `Context` structs + per-tier dyn-compatible `Procedure` traits | `_spec/05` §3 |
| D18 | Pack lifecycle audit via the same mutation funnel. **Revised (BRIEF-Item-7):** lifecycle events remain in the WAL change stream **and** are mirrored to a dedicated `audit.log` ([D24](#)) with independent retention, so they survive WAL-archive pruning (Seam D). The original "no parallel ledger" framing is superseded by "single writer per concern": one funnel writes both, WAL-first / audit-after (audit can lag, never lead). | `_spec/05` §5; `_design/deletion-reclamation-audit.md` Item 7 |
| D19 | GG02 catalog: per-graph immutable `bound_type: Option<Arc<GraphTypeDef>>` runtime binding, persisted via `CORE/GTYP`. **Amended (JSON/L1c-d):** the catalog now persists closed/typed **and** open RECORD property types on **both** type-models — rkyv `RecordFieldTypes` in `CORE/GTYP` (authoritative) + serde `RecordFieldStructure` on the WAL `PropertyDef.record_fields`. `RecordFieldStructure` is `enum { Open, Closed(Vec<..>) }`; with the `Option` it encodes three durable states (`None` = not-record / `Some(Open)` = bare RECORD / `Some(Closed)` = typed). The open marker is **load-bearing** — without it WAL replay degrades a bare RECORD to `Null`. `ValueType.record` stays dormant (structural-inline, O4). GV46/47/48 are SUPPORTED; a closed-record `CAST` field-mismatch raises new GQLSTATUS **22G0U** (commit-time GG02 record violations use the existing G2000 class). | `_spec/02` §6, `_spec/03` §3; JSON/L1c-d |
| D20 | `Value::ExternalString(Arc<str>)` is the canonical surface for engine-produced strings that must NOT enter the global `IStr` pool. `PartialEq` is variant-strict; cross-variant string equality lives at the executor layer (`selene-gql::runtime::value_compare`). | `_spec/15` E74 |
| D21 | Snapshot harness pattern: pure-mirror DSL in `selene-testing` (zero production-crate imports), renderer + integration test + golden `.snap` files in the target crate, `test-harness` feature + `[[test]] required-features` gate, drift-detection tests deriving coverage from observed execution. Dep direction is `target -> selene-testing` as dev-dep. | `_spec/13`, `_spec/14`, `_spec/15` E81–E84 |
| D25 | `ChangeSubscriber` trait: runtime and recovery fan-out of `Change` events to extension providers via per-provider `ChangeKindSet` filter. Extension providers use it to tombstone derived state when graph nodes are deleted. Forward-compatible with all extension providers. | `_design/deletion-reclamation-audit.md` Item 1 |
| D26 | Snapshot + WAL-archive retention: typed `RetentionPolicy { keep_n_snapshots, keep_n_wal_archives, max_total_size_bytes: Option<u64>, time_based: Option<Duration> }` (embedder **config**, deliberately *not* persisted in the MANIFEST — avoids a divergent second source of truth) + `selene_persist::prune(dir, policy)` / `WalWriter::prune`. Prune is MANIFEST-atomic (rewrite-then-delete; the live snapshot + active WAL are never deletable regardless of policy) and reclaims superseded crash-orphans. The four constraints are conjunctive. Defaults: keep 2 snapshots / 4 archives, no size/time cap. | `_design/deletion-reclamation-audit.md` Item 5 |
| D24 | Dedicated append-only `audit.log` (`SLAU`) for engine-owned events, with retention independent of the WAL/snapshot lineage. **Surgical scope (BRIEF-Item-7):** carries pack-lifecycle events (Seam D) + a forward framework for user-action audit; records are generic `kind`-tagged opaque payloads (selene-persist stays below lifecycle semantics + clock — the caller stamps `recorded_at_unix_nanos`). The D12 per-commit principal **stays in the WAL entry header** (no format break, no hot-path write-amplification) — full principal relocation was deliberately *not* taken. `AuditLog` (open/append/read_all/prune, torn-tail-truncating scan) + `AuditRetentionPolicy { keep_n_events, max_age }` (atomic read-filter-rewrite compaction; default unbounded). Mirrored through the graph funnel (`DurableState::with_audit_log`, `SharedGraphBuilder::with_audit_log`), WAL-first/audit-after; recovery reattaches by file presence. | `_design/deletion-reclamation-audit.md` Item 7 |
| D22 | `NodeId`/`EdgeId` split into external stable **monotonic `NodeId(u64)`/`EdgeId(u64)`** (never-reused counter allocated under the D10 write lock; `*Id::TOMBSTONE == 0`, so id 0 is unallocatable and a safe hole sentinel) + internal **`RowIndex(u32)`** (dense row-column position, remappable by 4b/4c compaction). Reads resolve external↔row through `node_id_to_row`/`edge_id_to_row` maps + a `row_to_id` reverse column — never `id±1` arithmetic (enforced by `.github/scripts/check-no-rowid-arith.sh`, CI + pre-commit). PropertyMap/LabelSet columns are row-indexed. **Edges + adjacency still key endpoints by external `NodeId`** — the internal-`RowIndex` swap there is deferred to 4b. Snapshot persists explicit ids + positional recovery (see D14). | `_design/deletion-reclamation-audit.md` Item 4; BRIEF-Item-4a (PR #190) |

## Forward decisions — v1.x reclamation cycle (planned, not yet shipped)

Following the 2026-05-26 deletion + reclamation audit (`_design/deletion-reclamation-audit.md` — five parallel research passes across in-memory graph / WAL / snapshot/recovery / GQL), the project commits to a long-term-correctness reclamation cycle. Standing top priority going forward (see memory `project_deletion_reclamation_cycle`). The audit doc is the canonical reference; Stage 0 dispatches for any of the 14 brief-shaped items below MUST cite that doc + ground against current HEAD before drafting.

**D-record amendments planned** (land in the indicated brief; do not pre-update D1–D21 rows until shipped):

| Existing D | Amendment | Lands in |
|---|---|---|
| ~~**D11** (no ID reuse)~~ | ✅ **Shipped (BRIEF-Item-4a), foundation:** *external* `NodeId`/`EdgeId` remain permanent & never-reused; the *internal* `RowIndex(u32)` is now decoupled (reads via id↔row maps, no `id==row+1`) so 4b/4c compaction can remap it. Actual cross-epoch row remapping lands in 4b/4c. Decision-log D11 row amended above. | BRIEF-Item-4a |
| 🟡 **D14** (rkyv snapshot archive) | **Partially shipped (BRIEF-Item-4a / STEP 9):** explicit-id positional `CORE/NODE`+`CORE/EDGE` encode + positional recovery + `SNAPSHOT_VERSION_MINOR` 0→1 clean break (pre-split snapshots rejected). **Remaining for 4c:** internal-id remap headers + dual decoder for pre-compaction snapshots. Decision-log D14 row amended above. | BRIEF-Item-4a + 4c |
| ~~**D15** (two-step recovery)~~ | ✅ **Shipped (BRIEF-148):** three-step **MANIFEST read** → snapshot apply → WAL replay; the MANIFEST commit-point makes multi-phase rotate crash-safe + adds parent-dir fsync. Decision-log D15 row updated above. | BRIEF-Item-2 / BRIEF-148 |
| ~~**D18** (pack lifecycle WAL-only)~~ | ✅ **Shipped (BRIEF-Item-7), surgical scope:** lifecycle events are mirrored to a dedicated `audit.log` with independent retention (Seam D fixed) *and remain in the WAL* (one funnel, WAL-first/audit-after). The D12 principal slot was **kept in the WAL entry header** — full principal relocation was deliberately not taken (no format break, no hot-path cost). Decision-log D18 row updated + D24 added above. | BRIEF-Item-7 |

**New D-records planned** (do not add rows until shipped):

| Planned ID | Decision | Lands in |
|---|---|---|
| ~~**D22**~~ | ✅ **Shipped (BRIEF-Item-4a, PR #190):** external monotonic `NodeId(u64)`/`EdgeId(u64)` + internal `RowIndex(u32)`, `node_id_to_row`/`edge_id_to_row` maps + `row_to_id` column, all reads decoupled from `id±1` (grep gate), explicit-id positional snapshot encode + recovery. Edge/adjacency `RowIndex` swap deferred to 4b. Decision-log D22 row added above. | BRIEF-Item-4a |
| **D23** | `StorageCompactor` trait: every storage provider (selene-graph, future selene-timeseries / selene-rdf) implements `compact_for_snapshot(live_ids) -> CompactionResult`. Snapshot publication runs all compactors atomically under the MANIFEST epoch. Cross-storage, forward-compatible with all extensions. | BRIEF-Item-4b |
| ~~**D24**~~ | ✅ **Shipped (BRIEF-Item-7):** separate append-only `audit.log` (`SLAU`) for engine-owned events (pack lifecycle now + framework for future user-action audit) with independent `AuditRetentionPolicy`; same funnel writes WAL + audit (WAL-first/audit-after). Scoped down from the original plan: the D12 principal stays in the WAL header (not relocated). Decision-log D24 row added above. | BRIEF-Item-7 |
| ~~**D26**~~ | ✅ **Shipped (BRIEF-Item-5):** typed `RetentionPolicy` (config, not persisted) + MANIFEST-atomic `prune`; live snapshot + active WAL never deletable; conjunctive count/size/time constraints; reclaims crash-orphans. Decision-log D26 row added above. | BRIEF-Item-5 |

Brief sequence (14 work items, 4 tiers) lives in `_design/deletion-reclamation-audit.md`. Release allocation is per `project_deletion_reclamation_cycle` memory.

**Direction shift 2026-05-30:** selene-db is consolidating to a single native graph engine with no opt-in extensions. The vector store is externalized to a separate dedicated project and planned full-text/BM25 is dropped entirely; algorithms will be inlined natively and the extension-pack system removed in a following workflow.

## Where state lives

| Surface | Canonical home | Working view |
|---|---|---|
| Architecture decisions | `_spec/*` (gitignored; reproducible from MCP) | This file's decision-log row |
| Milestone progress | MCP graph (`selenedb` instance, project node 364) | `_design/milestone-log.md` (gitignored) |
| Brief status / fold records / PR links | Brief nodes under the milestone | `_briefs/NNN-<slug>.md` §O fold table (gitignored) |
| Recent code history | `git log` | — |
| Standing user preferences / agent memory | `~/.claude/projects/.../memory/` (Claude) | `~/.codex/memories/` (Codex) |
| Universal partner-workflow contract | `/Users/justin/Development/_agent_helpers/partner-workflow.md` | `~/.codex/AGENTS.md` (executor-side condensation) |

If the in-tree `_design/milestone-log.md` view drifts from the graph, regenerate it from the graph (the graph wins).

## Build & test

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --all-features --profile default
cargo test --workspace --locked --all-features --doc
cargo deny check bans licenses sources
cargo audit -d /private/tmp/selene-advisory-db
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
bash .github/scripts/check-thirdparty-current.sh
cargo +nightly fuzz run parse_gql -- -max_total_time=60

# Optional local speed knobs:
# CARGO_PROFILE_TEST_DEBUG=0 mirrors CI's stripped test debuginfo.
# PROPTEST_CASES=64 shortens inner-loop runs for the two 256-case proptests.

# Benchmarks: serialized runner only. PR CI checks invocation hygiene
# but does not execute benchmarks; run Criterion locally.
# iai-callgrind requires Linux/valgrind.
# Bench binaries use mimalloc as the global allocator; library crates are
# allocator-agnostic. See _design/perf-baselines.md §3.6.
scripts/run-benches.sh --profile quick --layer criterion
scripts/run-benches.sh --profile full  --layer criterion
scripts/run-benches.sh --profile quick --layer iai

# Forbidden: workspace bench execution can run bench binaries in parallel.
# cargo bench --workspace
```

### CI topology & local hooks (2026-05-28)

Three workflows + local git hooks split the gates by cost so Brief PRs iterate cheaply:

- **`.github/workflows/ci.yml`** — fast gate on **PR → `development`**. *No Rust build*: `fmt`, file-size cap, no-secret scan, bench-invocation lint, and (deps-changed only) `cargo-deny` + THIRDPARTY currency.
- **`.github/workflows/release.yml`** — full gate on **PR → `main`** (the `development` → `main` release PR): clippy + tests on ubuntu **and** macOS + doctests + `cargo-audit` + a 5-min parser fuzz, on top of the fast checks.
- **`.github/workflows/nightly.yml`** — schedule + push-to-`main`: advisory-DB drift + 1h fuzz soak.
- **Local hooks** (`scripts/install-hooks.sh` sets `core.hooksPath=.githooks`): `pre-commit` = fmt + file-size + secrets; `pre-push` = fast `clippy -D warnings` (lib/bin only). The full `nextest` + doctest suite intentionally does **not** run on every push — it runs inside agent workflows and at the `development` → `main` release gate (`release.yml`). Skip with `--no-verify` or `SELENE_SKIP_HOOKS=1`; run `install-hooks.sh` once per clone. rust-analyzer (LSP) is editor-side — enable clippy-on-save.

The `cargo …` block above is the full local set; the pre-push hook runs a fast clippy lint so `development` stays compiling without per-PR CI build minutes, while the full nextest/doctest suite runs at the `development` → `main` release gate (and inside agent workflows before they push).

## Conventions

- **Commits:** conventional commits (`feat(scope):`, `fix(scope):`, `refactor(scope):`, `chore(scope):`); scope = crate or component.
- **Branches:** `development` is the integration trunk. Brief work — `feature/*` / `feat/*` / `chore/*` → PR → **`development`** (fast checks-only gate). Releases batch `development` → PR → **`main`**, where the full suite runs; release tags cut from `main`. Don't PR Brief work straight to `main`.
- **Tests with code:** every PR ships extensive tests — units, edge cases, error paths, concurrency for shared state, property tests for invariants. Bar is "would this catch the IStr admission race."
- **Decisions over guesses:** when blocked, surface the question with discrete options and a recommendation. Don't pre-commit.

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

Detailed research notes are in agent memory (`project_lessons_from_archaeology.md`, `project_donor_perf_audit.md`).

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

Top-level directories beginning with `_` (e.g., `_spec/`, `_briefs/`, `_review/`, `_design/`, `_plan/`) are **local-only working documents** and excluded from git via `.gitignore`'s `_*/` pattern. Treat them as API-secret-grade: never `git add -f`; audit `git diff --cached` for `^_` before every commit.

Canonical durable state for design decisions, milestones, work items, and prior art lives in the **MCP graph** (selenedb instance — project node 364, label `selene-db`). The underscore-prefixed folders are dense markdown views of that state plus working scratchpads (briefs to agents, review reports, draft specs).

Implications:

- Tracked files (Cargo.toml, source under `crates/`, AGENTS.md, CLAUDE.md, LICENSE-*, NOTICE, THIRDPARTY.md, deny.toml, CI workflows, etc.) ship in git.
- Untracked working documents (`_spec/*`, `_briefs/*`, `_review/*`, `_design/*`, `_plan/*`) live on the local filesystem and are reproducible from the graph.
- When briefing an agent, specs are referenced via absolute filesystem paths, NOT relative-to-checkout paths, because a fresh `git clone` doesn't include them.
- The MCP graph is the source of truth. If a working document drifts from the graph, the graph wins; regenerate the working document.
