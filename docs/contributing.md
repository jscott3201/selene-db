# Contributing to selene-db

This document is for engineers contributing code to selene-db. It collects
the engineering posture, the local toolchain, the build / test / lint
commands, the CI gates that run on every pull request, and the PR
workflow. For the design rationale behind the workspace shape, read
[`architecture.md`](architecture.md) alongside this guide.

selene-db is an embeddable Rust property graph engine. It is library-only:
no server, no transport, no auth, no wire format. Embedders take the
workspace crates as dependencies and run the engine in-process. That
posture, combined with a strict GQL language boundary, shapes the contribution
rules below. Formal 2.0 wording is evidence-gated; see the
[conformance policy](v2/conformance-policy.md).

All package and release work follows the
[2.0 line and 1.x end-of-life policy](v2/eol-and-version-policy.md). Do not add
1.x fixes, compatibility shims, persisted-store readers, migrators, releases,
or tags. The alpha version in source is not proof of crates.io publication.

---

## 1. Engineering posture

The non-negotiable floors below are codified by workspace lints and repository
gates. The [finalized 2.0 decisions](v2/decisions/finalized.md) are the program
authority; older local decision labels in historical documents are not.

| Floor | Mechanism |
| :--- | :--- |
| No `unsafe` in selene-db source | `unsafe_code = "forbid"` at workspace level. |
| Every `pub` item carries rustdoc | `missing_docs = "deny"` at workspace level. |
| 700 LOC per file cap | CI gate `file-size cap (700 LOC)`. |
| rustls-only TLS posture | `cargo-deny` deny-list on `native-tls`, `openssl-sys`, `openssl-src`, `schannel`, `security-framework`. |
| No hand-rolled crypto, TLS, async runtime, or serialization primitives | Delegate to upstream crates: `blake3`, `xxhash-rust`, `rkyv`, `postcard`, `jiff`, `rust_decimal`. |
| Conventional commits with crate-or-component scope | `type(scope): subject`, for example `feat(selene-gql): ...` or `fix(BRIEF-NN): ...`. |
| Library only | No server, transport, or bundled authentication/authorization service. |
| Strict GQL boundary | No Cypher, SQL, or SPARQL grammar in the parser. |

The codebase contains zero `unsafe` blocks of its own. Donor code adapted
from prior forks is scrubbed of `unsafe` before integration; where a fast
path required `unsafe` upstream, selene-db ships the safe equivalent or
declines the pattern.

The 700 LOC cap is the only per-source budget that gates merges. Brief
acceptance bars never set per-crate budgets. Files that approach the
limit are split by module, not by removing whitespace.

---

## 2. Setup

### Rust toolchain

The workspace pins a single stable Rust release:

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.97.1"
components = ["rustfmt", "clippy"]
profile = "default"
```

`Cargo.toml` sets `rust-version = "1.97.1"` and `edition = "2024"`. Use
the pinned toolchain locally. `rustup` picks up `rust-toolchain.toml`
automatically when you `cd` into the workspace.

### Required tools

| Tool | Purpose | When you need it |
| :--- | :--- | :--- |
| `cargo` | Build, test, lint. | Always. |
| `rustfmt` | Formatting. | Always (installed by `rust-toolchain.toml`). |
| `clippy` | Lints. | Always (installed by `rust-toolchain.toml`). |
| `cargo-nextest` 0.9.143 | Workspace test runner used by CI. | Before handing off code changes. |
| `cargo-deny` | License / ban / source allow-list. | Locally before opening a PR (CI also runs it). |
| `cargo-audit` | Vulnerability advisories. | Locally before opening a PR (CI also runs it). |
| `cargo-about` 0.9.2 | `THIRDPARTY.md` generation. | After dependency changes; CI gates drift and tool-version mismatches. |

### Optional tools

| Tool | Purpose |
| :--- | :--- |
| `cargo-fuzz` (nightly) | Run the GQL parser fuzz harness. Release CI runs it on Linux; local fuzzing is risk-driven. |

Install the cargo helpers:

```bash
cargo install cargo-nextest --version 0.9.143 --locked
cargo install cargo-about --version 0.9.2 --locked --features cli
cargo install cargo-deny cargo-audit
# Nightly + cargo-fuzz are only needed for local fuzz runs.
rustup toolchain install nightly
cargo install cargo-fuzz
```

---

## 3. Building

The workspace compiles cleanly with the default feature set:

```bash
cargo check --workspace
```

For a faster inner loop on a single crate:

```bash
cargo check -p selene-gql
cargo build -p selene-graph --tests
```

Release-profile rebuilds are slow because of `lto = "thin"` and
`codegen-units = 1`. For day-to-day work, stay on the dev profile.

---

## 4. Testing

The full test suite runs across the workspace:

```bash
cargo test --workspace --all-features
```

Every non-draft development PR runs all-features workspace `cargo check` and
the CI-profile nextest suite on Linux. Run the owning work item's focused and
risk gates locally before handoff; release PRs add the comprehensive
Linux/macOS, clippy, doctest, audit, attribution, and fuzz coverage.

Per-crate test runs are fine for tight iteration:

```bash
cargo test -p selene-gql parser
cargo test -p selene-graph --test mutator_concurrency
```

### Test discipline

Every PR ships with tests. The bar is units, edge cases, error paths,
concurrency tests where state is shared, and property tests where
invariants are checkable.

The workspace uses four kinds of tests:

- **Unit tests** colocated with the implementation (`#[cfg(test)] mod tests`).
- **Integration tests** under `crates/<crate>/tests/`.
- **Property tests** via `proptest` for invariants such as parser
  round-trips, codec symmetry, and persistent-collection equivalence.
- **Snapshot tests** via `insta` for any output that must not drift
  silently (planner, executor, built-in procedure signatures, algorithm
  result shapes, recovery results).

See [`architecture.md`](architecture.md#7-snapshot-harness-pattern) for the
snapshot-harness pattern and pure-mirror invariant.

---

## 5. Formatting and lints

Format with the workspace `rustfmt.toml`:

```bash
cargo fmt --all
```

Lint everything clippy sees, with warnings denied:

```bash
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`Cargo.toml` configures the workspace-wide lint floors:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "deny"
unused_must_use = "deny"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
todo = "deny"
dbg_macro = "deny"
print_stderr = "warn"
print_stdout = "warn"
```

The `-D warnings` flag on the CLI promotes every clippy warning to an
error in CI. The workspace `[lints]` block carries the pedantic / nursery
opt-ins; do not paper over a warning with `--allow` on the command line.
If a lint is genuinely wrong for a site, add a narrow `#[allow(...)]`
with a comment that explains why.

Install the project-managed hooks once with `scripts/install-hooks.sh`.
`.githooks/pre-commit` mirrors cheap development-PR checks, while
`.githooks/pre-push` runs workspace clippy. Workspace nextest also runs in the
development CI lane rather than in the push hook.

---

## 6. CI gates

The development workflow lives at
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). It runs on non-draft
PRs to `development` and on manual dispatch. Rust compilation/tests and the 2.0
plan contract are unconditional; dependency gates remain path-conditional.

| Gate | What it checks |
| :--- | :--- |
| `fmt` | `cargo fmt --all --check` on Ubuntu. |
| `file-size cap (700 LOC)` | `.github/scripts/check-file-size.sh` counts non-empty, non-comment lines in every tracked `*.rs` file and fails if any exceeds 700. |
| `no-secret scan` | `.github/scripts/check-no-secrets.sh` greps tracked files for AWS access key ids, private-key blocks, Slack tokens, GitHub tokens, and `sk-`-prefixed API tokens. |
| `bench invocation lint` | `.github/scripts/check-bench-invocation.sh`, `.github/scripts/check-mimalloc-dev-dep.sh`, and `scripts/run-benches.test.sh` together verify that no workflow or shell script invokes `cargo bench --workspace` or wraps `cargo bench` in a parallel runner, and that `mimalloc` stays a dev-only dep. |
| `rust compile and test` | All-features workspace `cargo check`, then all-features workspace nextest with the `ci` profile on Ubuntu. |
| `2.0 plan contract` | The positive plan validator and its committed offline negative tests. |
| `doc constants` | `.github/scripts/check-doc-constants.sh` checks source-count claims. |
| Dependency gates | `cargo-deny` and third-party attribution run when manifests change. |

The comprehensive release workflow for PRs to `main` owns clippy, nextest,
doctests, audit/deny, attribution, macOS, and fuzz coverage. Clippy remains in
the pre-push, full local, and release gates rather than duplicating it in routine
development CI. Local work-item gates remain required.

After M00-PR03 merges, the desired `development` required contexts are exactly
`fmt`, `file-size cap (700 LOC)`, `no-secret scan`, `bench invocation lint`,
`rust compile and test`, and `2.0 plan contract`. Authenticated settings before
this PR have only the first four and no required-review rule. Adding the new
contexts is a post-merge owner/settings action; repository files do not mutate
branch protection. The independent reviewer pair is the 2.0 review control, so
no impossible GitHub self-approval is required.

### Benchmarks are not a CI gate

Benchmarks run **locally only**. There is no benchmark job in CI. The
runner script [`scripts/run-benches.sh`](../scripts/run-benches.sh)
serializes bench binaries across the workspace because Cargo may
otherwise dispatch them concurrently and corrupt measurements. The
`bench invocation lint` gate exists to make sure no script or workflow
re-introduces a parallel invocation.

To run benches locally:

```bash
# Curated smoke profile.
scripts/run-benches.sh --smoke

# One full-profile Criterion target.
scripts/run-benches.sh --profile full --bench single_graph

# Filter one target by Criterion ID.
scripts/run-benches.sh --profile quick --bench single_graph --filter node_fetch
```

The runner is Criterion-only. Registry, command, and current evidence records
live in [`BENCHMARKS.md`](../BENCHMARKS.md).

---

## 7. Submitting a pull request

### Branch naming

Use any branch name that identifies your work. Brief-driven branches
follow `brief-NN-short-slug`; one-off changes can use any short
hyphenated slug.

### Commit messages

Use conventional commits with a crate-or-component scope:

```text
feat(selene-gql): add OPTIONAL-MATCH null padding for typed bindings
fix(selene-persist): cover NewDecoderErrors in recovery match arm
chore(BRIEF-NN): close milestone after merge
docs(v2): install the program contract
```

Allowed types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`.

The scope is the crate name (`selene-graph`, `selene-algorithms`, ...) or a
component identifier (a brief id, an area like `ci` / `bench` /
`scripts`). A subject line should fit on one line; longer rationale
goes in the commit body.

### PR title and description

Open the PR with a one-line title in the same conventional-commits
shape as the head commit. The PR description should cover:

- **What** changed at a one-paragraph level.
- **Why** the change is needed (or which brief id it implements).
- **How** to verify locally (test commands, scripts, bench invocations
  if perf-sensitive).
- **Risk** notes for anything beyond a localized change: format
  changes, public-API changes, recovery paths, lock orderings,
  provider re-entry boundaries.

CI runs automatically on PR open and on every push to the branch. Every
required applicable gate and check must be green before the PR can merge.

The repository PR template collects the complete 2.0 handoff. It records
evidence and role confirmations but does not technically enforce them.

### Review

Every merge requires green required checks and review under the repository's
applicable policy. For 2.0 work, follow the
[PASS/FIX/REPLAN protocol](v2/review-protocol.md): the implementer edits and
tests only; the orchestrator owns Git history, the non-draft PR, consolidated
comments from two independent read-only reviewers, and any eligible authorized
merge. Merge eligibility requires an unchanged reviewed head, green exact-head
required checks, Blocker/Major-clean final review, policy permission, clean
scope/worktree state, and explicit user authorization. A changed head voids
PASS. This does not authorize self-approval, auto-merge, release, publication,
tagging, reactions, or branch-protection changes.

---

## 8. 2.0 program decisions

The canonical architecture and governance authority is
[`docs/v2/decisions/finalized.md`](v2/decisions/finalized.md). Read the owning
work item with those decisions before editing. A conflicting current-source fact
returns REPLAN; it is not permission to reinterpret a decision or preserve a
superseded 1.x contract.

---

## 9. Snapshot harness pattern

Every runtime surface that can drift silently is pinned by golden
`.snap` files: planner output, executor output, built-in procedure
signatures, algorithm result shapes, recovery results. The pattern is:

1. **Pure-mirror DSL** in `selene-testing` expressing the producer's
   public output shape as serializable structs. The mirror crate's
   `[dependencies]` MUST NOT include the target crate, otherwise the
   golden snapshots re-render automatically on every refactor and
   the drift signal vanishes.
2. **Renderer** in the target crate that builds the mirror summary
   from real engine output.
3. **Integration test** that fans the renderer over a representative
   corpus and asserts against committed `.snap` files via `insta`.

For mirrors over `#[non_exhaustive]` foreign enums, the pattern pins
the shape via three mechanisms: explicit `use foreign::Enum::{...}`
imports (a removed variant fails to compile), a `const ANCHOR` table
enumerating expected names + counts, and a coverage test that exercises
each anchor row.

When you add a snapshot golden, accept the rendered output via
`cargo insta review` and commit the `.snap` file alongside the test.
See [`architecture.md`](architecture.md#7-snapshot-harness-pattern)
for the full discussion.

---

## 10. Adding a new feature gate

At the c5 baseline, `selene-core` carries the parser-visible feature inventory.
It does not establish a formal 2.0 claim. M01 replaces claim authority with the
generated profile described by the
[conformance policy](v2/conformance-policy.md). Until that cutover, keep runtime
and parser behavior synchronized with the current register and update evidence
without strengthening public claim wording.

If you implement an ISO optional feature:

1. Add the register entry in `selene-core` with the ISO feature id
   (for example `GG02`, `GT01`, `IW010`).
2. Wire the parser and analyzer to admit the construct only when the current
   register reports it implemented.
3. Add positive corpus entries under `crates/selene-testing/corpus/positive`
   and negative corpus entries under `crates/selene-testing/corpus/negative`
   if the construct has rejectable variants.
4. Update [`gql-reference.md`](gql-reference.md) with the surface that
   is now usable.

If a feature is **not** ISO (a selene-db extension, such as
backtick-delimited identifiers), call it out explicitly in the
implementing module's rustdoc as a deliberate non-ISO extension. The
Flagger must still admit it; do not pretend it is standard.

---

## 11. Adding a `CALL` procedure

selene-db is a single native engine: there is no procedure-pack model.
All `CALL`-able procedures are registered natively in the one frozen
`selene_gql::runtime::builtin_registry::BuiltinProcedureRegistry` —
the 50 `selene.*` platform built-ins plus the 19 `algo.*` procedures.

The high-level flow is:

1. Decide the tier: read-tier procedures get a `GraphContext`,
   write-tier procedures get a `MutationContext`. The planner enforces
   tier compatibility at plan time.
2. Author the procedure body as a native function over its `Context`
   plus row-shaped inputs that returns row-shaped outputs. For an
   algorithm, add the native free function to `selene-algorithms` first
   (see [`graph-algorithms.md`](graph-algorithms.md) §11.1).
3. Register the procedure in `BuiltinProcedureRegistry`, mirroring an
   existing entry for the argument-coercion and YIELD-column contract.
   The registry is frozen (`registry_version()` constant `0`); the set
   is fixed at construction.

Cross-reference [`graph-algorithms.md`](graph-algorithms.md) §11.2 for a
worked example of exposing a new algorithm through GQL `CALL`.

---

## 12. Code style

Clippy with `-D warnings` is the floor. Beyond that:

- Prefer named functions to closures for anything that does not fit
  on one line.
- Prefer explicit error types (`thiserror` derive) to `anyhow` for
  library-surface errors. `miette` is wired up workspace-wide for
  diagnostic surface.
- Prefer `Result<T, E>` returns over panicking constructors. Panics
  are a tool for "the caller violated a documented precondition," not
  for input validation.
- Prefer borrowed slices to owned `Vec` arguments where the callee
  does not need ownership.
- Prefer `let _ = ...;` over `#[allow(unused_must_use)]`; the
  workspace denies `unused_must_use`.
- Tests are not optional. A PR that adds public surface without tests
  will not pass review.

### parking_lot and the lock-binding rule

`parking_lot::Mutex` is non-reentrant. `match &*mutex.lock() { ... }`
holds the guard across the entire match. If an arm mutates
`*mutex.lock()`, the second lock deadlocks. Bind the guard to a named
local before matching:

```rust
let guard = mutex.lock();
match &*guard {
    State::Idle => {
        drop(guard);
        // safe to re-acquire here
    }
    State::Active(payload) => { /* ... */ }
}
```

---

## 13. License headers

selene-db's own source files do **not** carry license headers; the
workspace ships under MIT OR Apache-2.0 at the embedder's choice via
`LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE`.

When a file is **adapted** from a third-party upstream, the adapted
file carries a single-line attribution comment at the top:

```rust
// Adapted from <upstream-name>@<version-or-commit> (<SPDX>)
```

Examples:

```rust
// Adapted from foo-rs@v2.1.3 (MIT)
// Adapted from bar-crate@1f2a3b4 (Apache-2.0)
```

The attribution is required even for small adaptations. `NOTICE`
collects copyright holders for bundled or adapted code; add an entry
there when you introduce a new adapted source.

`THIRDPARTY.md` is generated from `Cargo.lock` by cargo-about 0.9.2:

```bash
cargo about generate about.hbs | sed 's/[[:space:]]*$//' > THIRDPARTY.md
```

Do not hand-edit it. CI fails the `third-party attribution current` gate on a
tool-version mismatch or output drift.

---

## 14. What NOT to add

Some changes are out of scope:

- **No server, transport, or bundled authentication/authorization service.**
  selene-db is library-only; embedders own credentials, network identity, and
  policy through facade hooks. ISO 39075 Clause 4.2.3 puts no wire format in
  scope.
- **No graph types in `selene-persist`.** The persistence
  crate sees `&[Change]` going in and a `RecoveryResult` coming out.
  It must never grow a dependency on `selene-graph` or `selene-core`'s
  graph-shaped types.
- **No extension-pack split.** Vectors, text, JSON, indexes, algorithms, and
  native procedures are in-tree engine capabilities. Keep storage ownership in
  `selene-graph`, algorithms in `selene-algorithms`, and `CALL` registration in
  the native `BuiltinProcedureRegistry` in `selene-gql`.
- **No `unsafe` Rust** anywhere in selene-db's own source. The
  lint is `forbid`, not `deny`; you cannot override it locally. If a
  performance path seems to need `unsafe`, escalate the design in the
  PR description before writing the code.
- **No hand-rolled crypto, TLS, async runtime, or serialization
  primitives.** Delegate to `blake3`, `xxhash-rust`, `rkyv`,
  `postcard`, `jiff`, `rust_decimal`. The engine reserves the right
  to vendor a dependency if needed, but does not reimplement these
  surfaces.
- **No Cypher / SQL / SPARQL grammar in the parser.** The query
  language is ISO/IEC 39075:2024 GQL. Constructs outside the current
  generated profile marks as Flagger-rejected are rejected at parse time. Change
  `spec/gql-profile/profile.json`, regenerate with
  `cargo run --locked -p selene-db-profile --bin selene-profile -- --write`, and
  update the matching behavior evidence together.
- **No `cargo bench --workspace`** in any script or workflow. Use
  `scripts/run-benches.sh` so bench binaries run sequentially. The
  `bench invocation lint` CI gate enforces this.
- **No native-tls, openssl-sys, schannel, or security-framework** in
  the transitive dependency closure. `cargo-deny` enforces the
  ban list.

---

## See also

- [`architecture.md`](architecture.md) — current crate layout, threading
  model, and persistence design.
- [`v2/README.md`](v2/README.md) — target decisions, milestones, work items,
  and review protocol.
- [`embedding-guide.md`](embedding-guide.md) — using selene-db as a
  library in an application.
- [`getting-started.md`](getting-started.md) — install, first query,
  common patterns.
- [`persistence-and-recovery.md`](persistence-and-recovery.md) — WAL
  and snapshot formats, recovery flow.
- [`gql-reference.md`](gql-reference.md) — the ISO GQL surface
  selene-db supports.
- [`graph-algorithms.md`](graph-algorithms.md) — the native
  `selene-algorithms` API and the `algo.*` `CALL` surface.
- [`performance.md`](performance.md) — benchmarks and tuning knobs.
