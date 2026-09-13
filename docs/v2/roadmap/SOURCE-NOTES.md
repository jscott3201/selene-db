# Sources, evidence and limits

## Source hierarchy

Current repository implementation and its live plan establish what exists. The supplied the finish plan package establishes the original decisions, 11-milestone/64-item organization and remaining intent. The supplied ISO establishes language semantics. `yet-more-skills` establishes the actual specialist instructions. New grouping, ordering, adoption policy and integration gates are recommendations from this review, not previously merged work.

The review used read-only GitHub connector reads on `development` and the skills repository’s main branch. URLs below are navigation references without revision bookkeeping. Re-ground the relevant files at implementation time if the branch advances; do not redo a landed outcome.

## Repository sources

**S01 — Live program and counts.** `docs/v2/README.md`, `docs/v2/roadmap/milestones.md`, `docs/v2/roadmap/work-items-00-04.md`, `docs/v2/roadmap/work-items-05-10.md`, `docs/v2/roadmap/plan.json`, and `Cargo.toml`. The live program is 11 milestones/65 work items; the inspected workspace is nine crates at 2.0.0-alpha.1, Rust 1.97.1. Navigation: `https://github.com/jscott3201/selene-db/tree/development/docs/v2` and `https://github.com/jscott3201/selene-db/blob/development/Cargo.toml`.

**S02 — Planning friction.** PR #1181, “docs(plan): split M04-PR02 Part 3 into 3A and 3B”, merged 2 September 2026 in the returned PR record. The explanation describes missing graph/GQL consumers in an exact path inventory; this is planning evidence, not a product performance measurement. Navigation: `https://github.com/jscott3201/selene-db/pull/1181`.

**S03 — Current candidate contract.** M04-PR02 in `docs/v2/roadmap/work-items-00-04.md`, including four sequential parts, exact inventories, bridge/deletion states and issue #1093 closure. Navigation: `https://github.com/jscott3201/selene-db/blob/development/docs/v2/roadmap/work-items-00-04.md`.

**S04 — Actual candidate representation.** `crates/selene-graph/src/candidate_set.rs`: sealed node/edge kinds, lower graph/generation and physical/workspace tokens, live pairing checks, remaining trusted_rows methods, stable-ID binding and graph-owned algebra. Navigation: `https://github.com/jscott3201/selene-db/blob/development/crates/selene-graph/src/candidate_set.rs`.

**S05 — Actual public facade.** `crates/selene-db/src/lib.rs` and `database.rs`: current in-memory construction, session/catalog API, stable handles, Value/GqlType bridges and documented mutation-indeterminate behavior. Navigation: `https://github.com/jscott3201/selene-db/blob/development/crates/selene-db/src/lib.rs` and `https://github.com/jscott3201/selene-db/blob/development/crates/selene-db/src/database.rs`.

**S06 — Existing transaction authority.** `crates/selene-db/src/transaction.rs`: detached state, mutation reservation, complete outer publication and outcome classification. It is the base for durability integration, not evidence that current facade commit already synchronizes a format-2 store. Navigation: `https://github.com/jscott3201/selene-db/blob/development/crates/selene-db/src/transaction.rs`.

**S07 — Seven current issue threads.** #1088 directory capability; #1092 composite uniqueness; #1093 raw row bitmaps; #1094 uniqueness rescans; #1097 JSON indexing; #1128 WAL watermark/poison outcome; #1137 read-hot map regression. Issue-reported benchmark ranges are not independently reproduced results. Navigation uses `https://github.com/jscott3201/selene-db/issues/<number>`.

**S08 — Current persistence implementation.** Inspected persistence tree and `crates/selene-persist/src/writer.rs`, including canonical path ownership, tail scanning/repair, committed_offset and flush→sync_data. Actual source uses `src/writer.rs`, not an assumed `src/wal/writer.rs`. Navigation: `https://github.com/jscott3201/selene-db/tree/development/crates/selene-persist/src`.

**S09 — Remaining compiler plan.** `docs/v2/roadmap/work-items-05-10.md`, specifically source/semantic separation, catalog resolution, type/effect/logical/physical layering and deletion ownership. This is planned architecture, not proof of new semantic modules already existing. Navigation: `https://github.com/jscott3201/selene-db/blob/development/docs/v2/roadmap/work-items-05-10.md`.

**S10 — Skills.** `README.md`, `catalog.json` and actual bodies of rust-storage-durability, rust-nextest, rust-test-design, tracer-bullet-planning and orchestrate in `jscott3201/yet-more-skills`. Other referenced skill names were verified in the catalog. Navigation: `https://github.com/jscott3201/yet-more-skills` and `https://github.com/jscott3201/yet-more-skills/tree/main/skills`.

**S11 — Current CI.** `.github/workflows/ci.yml`, plus release/nightly workflow inventory and documented lane responsibilities. The inspected PR workflow pins Rust 1.97.1 and nextest 0.9.143 and runs the all-feature workspace gate. Navigation: `https://github.com/jscott3201/selene-db/blob/development/.github/workflows/ci.yml`.

## Normative supplied source

**S12 — ISO/IEC 39075:2024(en), supplied PDF.** Relevant sections include graph/reference identity (§§4.3–4.4), binding tables (§4.3.6), transactions (§4.6 and clause 8), contexts/effects (§§4.7–4.10), mixed/path semantics (§4.11, §§16.3–16.12, §§22.2–22.4), types (§§4.12–4.17, clause 18), comparison/grouping (§§22.11–22.20), diagnostic outcomes (clause 23) and conformance (clause 24). Targeted reads included printed pp115–118 / PDF pp131–134 and printed pp513–516 / PDF pp529–532. The reference PDF is not bundled, quoted wholesale or turned into a redistributed grammar.

## Supplemental primary documentation

**S13 — Nextest.** Official running/filterset documentation: `https://nexte.st/docs/running/` and `https://nexte.st/docs/filtersets/`. These support test selection, profile and result-reporting guidance; the actual repo configuration remains authoritative for commands.

**S14 — Cargo packaging/publish.** Official command reference: `https://doc.rust-lang.org/cargo/commands/cargo-publish.html`. Dry-run does not upload; package/dependency checks do not replace consumer tests.

## What was and was not verified

The source/roadmap findings are based on the files and PR/issue records above. Proposed implementations, new module locations, API designs and delivery gates are recommendations. Paths explicitly labeled search hints were not verified as existing files; they are intended to be resolved by source discovery rather than treated as prescribed locations.

No Rust compiler/workspace test, benchmark, fuzz, crash campaign, platform qualification or downstream integration was run in this review. No remote repository write was made. The local package checks and supplemental Python reference tests are listed in PACKAGE-CHECK.md. Those results establish only the package/reference’s own behavior, not Selene’s implementation correctness or release readiness.
