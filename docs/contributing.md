# Contributing to selene-db

This document is for engineers contributing code to selene-db. It collects
the engineering posture, the local toolchain, the build / test / lint
commands, the CI gates that run on every pull request, and the PR
workflow. For the design rationale behind the workspace shape, read
[`architecture.md`](architecture.md) alongside this guide.

selene-db is an embeddable Rust property graph engine. It is library-only:
no server, no transport, no auth, no wire format. Embedders take the
workspace crates as dependencies and run the engine in-process. That
posture, combined with strict ISO/IEC 39075:2024 GQL conformance, shapes
the contribution rules below.

---

## 1. Engineering posture

The non-negotiable floors below are codified by workspace lints, CI gates,
and the numbered architecture decisions in [`architecture.md`](architecture.md#6-architecture-decisions-d1-d21).

| Floor | Mechanism |
| :--- | :--- |
| No `unsafe` in selene-db source | `unsafe_code = "forbid"` at workspace level (D9). |
| Every `pub` item carries rustdoc | `missing_docs = "deny"` at workspace level (D10). |
| 700 LOC per file cap | CI gate `file-size cap (700 LOC)` (D11). |
| rustls-only TLS posture | `cargo-deny` deny-list on `native-tls`, `openssl-sys`, `openssl-src`, `schannel`, `security-framework` (D20). |
| No hand-rolled crypto, TLS, async runtime, or serialization primitives | Delegate to upstream crates: `blake3`, `xxhash-rust`, `rkyv`, `postcard`, `jiff`, `rust_decimal`. |
| Conventional commits with crate-or-component scope | `type(scope): subject`, for example `feat(selene-gql): ...` or `fix(BRIEF-NN): ...` (D12). |
| Library only | No server, transport, or auth code anywhere in the workspace (D1). |
| Strict ISO GQL only | No Cypher, SQL, or SPARQL grammar in the parser (D2). |

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
channel = "1.95.0"
components = ["rustfmt", "clippy"]
profile = "default"
```

`Cargo.toml` sets `rust-version = "1.95.0"` and `edition = "2024"`. Use
the pinned toolchain locally. `rustup` picks up `rust-toolchain.toml`
automatically when you `cd` into the workspace.

### Required tools

| Tool | Purpose | When you need it |
| :--- | :--- | :--- |
| `cargo` | Build, test, lint. | Always. |
| `rustfmt` | Formatting. | Always (installed by `rust-toolchain.toml`). |
| `clippy` | Lints. | Always (installed by `rust-toolchain.toml`). |
| `cargo-deny` | License / ban / source allow-list. | Locally before opening a PR (CI also runs it). |
| `cargo-audit` | Vulnerability advisories. | Locally before opening a PR (CI also runs it). |
| `cargo-about` | `THIRDPARTY.md` generation. | After dependency changes; CI gates drift. |

### Optional tools

| Tool | Purpose |
| :--- | :--- |
| `cargo-fuzz` (nightly) | Run the GQL parser fuzz harness. CI runs it on Linux only; local fuzzing is optional. |
| `valgrind` | Required by `iai-callgrind` for the iai measurement layer. Linux only; macOS does not ship valgrind, and `scripts/run-benches.sh --layer iai` will refuse to run there. |

Install the cargo helpers:

```bash
cargo install cargo-deny cargo-audit cargo-about
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

CI runs `cargo test --workspace --locked --all-features` on both Ubuntu
and macOS. Match that locally before opening a PR.

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
  silently (planner, executor, procedure-pack metadata, algorithm
  result shapes, vector-index section bytes, recovery results).

See [`architecture.md`](architecture.md#7-snapshot-harness-pattern-d21)
for the snapshot-harness pattern (decision D21) and the pure-mirror
invariant.

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

There are no project-managed git hooks; configure your editor or a
personal `pre-commit` to run `cargo fmt` and `cargo clippy` if you want
local feedback.

---

## 6. CI gates

CI runs on every pull request against `main` and on `workflow_dispatch`.
The workflow lives at [`.github/workflows/ci.yml`](../.github/workflows/ci.yml).
Every gate below must be green before a PR can merge.

| Gate | What it checks |
| :--- | :--- |
| `fmt` | `cargo fmt --all --check` on Ubuntu. |
| `clippy (ubuntu-latest, macos-latest)` | `cargo clippy --workspace --all-targets --locked -- -D warnings` on both platforms. |
| `test (ubuntu-latest, macos-latest)` | `cargo test --workspace --locked --all-features` on both platforms. |
| `parse-fuzz` | `cargo +nightly fuzz run --target x86_64-unknown-linux-gnu parse_gql -- -max_total_time=60` against `crates/selene-gql/fuzz`, Linux only. The fuzzer is seeded from the positive GQL corpus in `crates/selene-testing/corpus/positive`. |
| `cargo-deny` | `cargo deny check bans licenses sources` against [`deny.toml`](../deny.toml): license allow-list, banned-crate list (rustls posture), source allow-list (crates.io only, git deps must be sha-pinned). |
| `cargo-audit` | `cargo audit` against the RustSec advisory database. Yanked crates fail. |
| `file-size cap (700 LOC)` | `.github/scripts/check-file-size.sh` counts non-empty, non-comment lines in every tracked `*.rs` file and fails if any exceeds 700. |
| `no-secret scan` | `.github/scripts/check-no-secrets.sh` greps tracked files for AWS access key ids, private-key blocks, Slack tokens, GitHub tokens, and `sk-`-prefixed API tokens. |
| `bench invocation lint` | `.github/scripts/check-bench-invocation.sh`, `.github/scripts/check-mimalloc-dev-dep.sh`, and `scripts/run-benches.test.sh` together verify that no workflow or shell script invokes `cargo bench --workspace` or wraps `cargo bench` in a parallel runner, and that `mimalloc` stays a dev-only dep. |
| `third-party attribution current` | `.github/scripts/check-thirdparty-current.sh` regenerates `THIRDPARTY.md` with `cargo about generate about.hbs` and fails on drift. |

Gates that depend on `Cargo.lock` (`cargo-deny`, `cargo-audit`,
`third-party attribution current`) short-circuit to a no-op when the
lockfile is absent. Once the lockfile exists, they are unconditionally
enforced.

### Benchmarks are not a CI gate

Benchmarks run **locally only**. There is no benchmark job in CI. The
runner script [`scripts/run-benches.sh`](../scripts/run-benches.sh)
serializes bench binaries across the workspace because Cargo may
otherwise dispatch them concurrently and corrupt measurements. The
`bench invocation lint` gate exists to make sure no script or workflow
re-introduces a parallel invocation.

To run benches locally:

```bash
# Quick smoke profile, criterion only.
scripts/run-benches.sh --profile quick --layer criterion

# Full publish-quality profile, both layers (iai requires Linux + valgrind).
scripts/run-benches.sh --profile full --layer both

# Filter to a single bench by name.
scripts/run-benches.sh --profile quick --filter graph_node_fetch
```

Trend tracking lives in committed perf-baseline documents under
[`_design/`](../_design) and in [`BENCHMARKS.md`](../BENCHMARKS.md).
There is no gh-pages dashboard.

---

## 7. Submitting a pull request

### Branch naming

Use any branch name that identifies your work. Brief-driven branches
follow `brief-NN-short-slug`; one-off changes can use any short
hyphenated slug.

### Commit messages

Conventional commits with a crate-or-component scope (decision D12):

```text
feat(selene-gql): add OPTIONAL-MATCH null padding for typed bindings
fix(selene-persist): cover NewDecoderErrors in recovery match arm
chore(BRIEF-NN): close milestone after merge
docs(architecture): document D21 snapshot harness mechanics
```

Allowed types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`.

The scope is the crate name (`selene-graph`, `selene-pack`, ...) or a
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

CI runs automatically on PR open and on every push to the branch. The
PR cannot merge until every gate is green.

### Review

Every merge requires CI green plus an external review. Self-review by
the author is required (walk the diff once after CI finishes) and an
external reviewer must sign off via the GitHub review surface. Inline
findings are resolved per-thread via reply, not by blanket dismissal.

---

## 8. Architecture decisions (D1-D21)

The workspace shape is codified by twenty-one numbered decisions. They
are the canonical reference for "why is this crate split this way" and
"why does the engine refuse to do X." See
[`architecture.md`](architecture.md#6-architecture-decisions-d1-d21)
for the full list. The most contribution-relevant decisions are:

| Decision | Subject |
| :--- | :--- |
| D1 | Library only; no server, transport, or auth. |
| D2 | Strict ISO GQL parser; no Cypher / SQL / SPARQL. |
| D5 | Non-graph capabilities live in extension crates, not in `selene-graph`. |
| D7 | Concurrency primitives: `ArcSwap`, `parking_lot`, `imbl`, `RoaringBitmap`, `triomphe`. |
| D8 | Multi-crate workspace, no umbrella facade. |
| D9 | `#![forbid(unsafe_code)]` workspace-wide. |
| D10 | `missing_docs = "deny"` workspace-wide. |
| D11 | 700 LOC per-file cap. |
| D12 | Conventional commits with crate-or-component scope. |
| D18 | Lifecycle audit through the mutation funnel; no parallel ledger. |
| D20 | rustls-only TLS posture. |
| D21 | Snapshot harness pattern for output drift. |

If you find yourself fighting one of these decisions, surface the
conflict in your PR description before reshaping the workspace.

---

## 9. Snapshot harness pattern (D21)

Every runtime surface that can drift silently is pinned by golden
`.snap` files: planner output, executor output, procedure-pack
signatures, algorithm result shapes, vector-index section bytes,
recovery results. The pattern is:

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
See [`architecture.md`](architecture.md#7-snapshot-harness-pattern-d21)
for the full discussion.

---

## 10. Adding a new feature gate

selene-db tracks ISO/IEC 39075:2024 GQL conformance through an explicit
feature register in `selene-core`. The Flagger (Clause 24.6) rejects
any construct outside the claimed register at parse time.

If you implement an ISO optional feature:

1. Add the register entry in `selene-core` with the ISO feature id
   (for example `GG02`, `GT01`, `IW010`).
2. Wire the parser and analyzer to admit the construct only when the
   register flags it claimed.
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

## 11. Adding a procedure to selene-pack

Procedure packs are JSON-manifest-validated, content-hashed bundles
registered into a frozen `ProcedurePackRegistry` at construct time
(D15, D16). External packs implement `ExternalProcedurePack` and
supply `ExternalGraphProcedure` (read-tier) or
`ExternalMutationProcedure` (write-tier) implementations.

The high-level flow is:

1. Decide the tier: read-tier procedures get a `GraphContext`,
   write-tier procedures get a `MutationContext`. The planner enforces
   tier compatibility at plan time (D17).
2. Author the procedure body. Procedures are pure functions over their
   `Context` plus row-shaped inputs that return row-shaped outputs.
3. Author the manifest entry. The manifest is validated against the
   JSON Schema 2020-12 schema in `selene-pack` with explicit gates
   (`MANIFEST_LEVEL_GATES`, `PROCEDURE_LEVEL_GATES`,
   `MANIFEST_VALIDATION_COVERAGE`, `FINAL_VALIDATION_COVERAGE`).
4. Reserve a 4-byte uppercase ASCII `ProviderTag` if you also need an
   `IndexProvider` (for instance, a new index family).
5. Register the pack with `selene-pack` at registry construction.

[`extension-guide.md`](extension-guide.md) is the full walkthrough:
manifest format, tier choice, the worked `hello.world` example, and
the registration patterns. Cross-reference
[`vector-search.md`](vector-search.md) and
[`graph-algorithms.md`](graph-algorithms.md) for two production
worked examples of pack-adapter crates (`selene-vector-pack` and
`selene-algorithms-pack`) that expose extension capabilities through
GQL `CALL`.

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

`THIRDPARTY.md` is generated from `Cargo.lock` by `cargo about
generate about.hbs > THIRDPARTY.md`. Do not hand-edit it. CI fails the
`third-party attribution current` gate on any drift.

---

## 14. What NOT to add

Some changes are out of scope by decision:

- **No server, transport, or auth code** anywhere in the workspace (D1).
  selene-db is library-only; embedders own the network and policy
  surfaces. ISO 39075 Clause 4.2.3 puts no wire format in scope.
- **No graph types in `selene-persist`** (D5, D14). The persistence
  crate sees `&[Change]` going in and a `RecoveryResult` coming out.
  It must never grow a dependency on `selene-graph` or `selene-core`'s
  graph-shaped types.
- **No vector or fulltext or timeseries types in `selene-graph`** (D5).
  Extension capabilities plug in through the `IndexProvider` trait and
  the procedure-pack registry. The graph crate ships pure graph
  storage; an embedder who wants neither extension does not depend on
  those crates.
- **No `unsafe` Rust** anywhere in selene-db's own source (D9). The
  lint is `forbid`, not `deny`; you cannot override it locally. If a
  performance path seems to need `unsafe`, escalate the design in the
  PR description before writing the code.
- **No hand-rolled crypto, TLS, async runtime, or serialization
  primitives.** Delegate to `blake3`, `xxhash-rust`, `rkyv`,
  `postcard`, `jiff`, `rust_decimal`. The engine reserves the right
  to vendor a dependency if needed, but does not reimplement these
  surfaces.
- **No Cypher / SQL / SPARQL grammar in the parser** (D2). The query
  language is ISO/IEC 39075:2024 GQL. Constructs outside the claimed
  feature register are rejected by the Flagger at parse time. If you
  want to admit a new optional feature, add the feature register
  entry first.
- **No `cargo bench --workspace`** in any script or workflow. Use
  `scripts/run-benches.sh` so bench binaries run sequentially. The
  `bench invocation lint` CI gate enforces this.
- **No native-tls, openssl-sys, schannel, or security-framework** in
  the transitive dependency closure (D20). `cargo-deny` enforces the
  ban list.

---

## See also

- [`architecture.md`](architecture.md) — crate layout, threading
  model, persistence design, D1-D21.
- [`embedding-guide.md`](embedding-guide.md) — using selene-db as a
  library in an application.
- [`getting-started.md`](getting-started.md) — install, first query,
  common patterns.
- [`persistence-and-recovery.md`](persistence-and-recovery.md) — WAL
  and snapshot formats, recovery flow.
- [`gql-reference.md`](gql-reference.md) — the ISO GQL surface
  selene-db supports.
- [`extension-guide.md`](extension-guide.md) — writing procedure
  packs and `IndexProvider` implementations.
- [`graph-algorithms.md`](graph-algorithms.md) — algorithm surface
  exposed through `algo.*` procedures.
- [`vector-search.md`](vector-search.md) — vector index extension
  surface exposed through `vector.*` procedures.
- [`performance.md`](performance.md) — benchmarks and tuning knobs.
