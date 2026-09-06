# Validation — fast local feedback, real integration, native release evidence

## Keep the existing posture

The inspected CI already distinguishes routine development PRs from heavier release/nightly qualification. Current non-draft development PRs run formatting, generated profile checks, workspace all-feature compilation/nextest and repository policy checks. Dependency/license/attribution checks are conditional where the current workflow allows. Release/nightly lanes add broader native, doctest, fuzz and security work. [S11]

Retain these requirements. The plan removes repetition of the entire release-style command list from every local iteration; it does not remove behavioral testing or let an agent claim an unrun gate passed. Any actual CI change is scoped to PLAN-01 or the owning release PR and must preserve required branch controls.

## Local loop

Use the repository wrapper before inventing an equivalent command. Read Cargo metadata, required features, test targets and `.config/nextest.toml`. The current observed floor is Rust 1.97.1 and CI pins nextest 0.9.143; keep them unless a real prerequisite requires a change. Do not turn this finish program into a toolchain/dependency refresh. Package names differ from crate directories: use cargo metadata rather than guessing `-p selene-graph` from its folder name.

A safe discovery command is:

```sh
cargo metadata --no-deps --format-version 1
```

Select the changed packages **and affected consumers**, with the actual feature configuration. For complex filters, list tests using the same selection before running them. Package selection controls what is built; an rdeps filter does not reintroduce packages excluded by `-p`. Fail a check that expected tests but selected none. Do not bypass ignored/hardware filters to inflate the count. [S10, S13]

Nextest profiles control runner behavior; `--profile ci` does not mean optimized Cargo code generation. Run doctests separately. A Criterion smoke iteration is not a benchmark measurement. Keep the first failure output and diagnose it rather than adding retries until green.

## Verified current PR commands

These are from the inspected CI, not an assertion they were executed in this review:

```sh
cargo fmt --all --check
cargo run --locked -p selene-db-profile --bin selene-profile -- --check
cargo check --workspace --locked --all-features
cargo nextest run --workspace --locked --all-features --profile ci
```

Also run the existing policy checks for affected paths: row-arithmetic/public API, no-secret, source size, profile feature errors, document constants and benchmark invocation. PLAN-01 updates the plan checker, not the runtime profile checker. Preserve compiler/linker errors and unsupported configuration warnings rather than suppressing them.

Public examples and API changes need separately selected doctests and documentation checks. Dependency changes need the existing lock/license/attribution/security treatment. Do not copy a developer-specific advisory-db filesystem path out of the old archive into a portable handoff.

## Risk-specific evidence

| Change | Fast owning-PR evidence | Joined milestone / RC evidence |
|---|---|---|
| Candidates/identity | Cross-domain, generation/layout, liveness and compile-fail API cases | Cross-crate query/algorithm/recovery behavior |
| Compiler/type/effects | Clause-derived positive/negative cases and independent small models | Full selected-profile/facade corpus and claim coverage |
| Batch execution | Batch-size boundary matrix, cardinality/null/resource cases | Complete query/control/native/path integration and old-runtime deletion |
| Paths | Independent bounded oracle and selector/mode witnesses | Broad mixed-graph/path corpus and resource campaigns |
| Indexes/constraints | Final-state model, activation completeness and delta-work checks | Replay/rebuild, batch/native writes and mixed workloads |
| Durable state | Deterministic I/O/phase failpoints plus real reopen | Native process-crash/corruption/retention stress and decoder fuzz |
| Performance | Controlled workload and correctness guards | Balanced native read/write/memory/recovery guard set |

## Native-only qualification

No QEMU and no cross-compilation locally or in GitHub. Native containers may be used on compatible host architecture. Test each supported native OS/filesystem behavior on an actual compatible runner; a Linux test does not establish macOS directory synchronization behavior. An unavailable required platform is an explicit qualification gap, not an automatic support downgrade or a fabricated pass.

Keep heavyweight fuzz/stress/crash/large-data matrices out of the ordinary edit loop unless the current gate requires them. Run them at the appropriate milestone/RC boundary and promote minimized failures into fast regression tests.

## Benchmarks

Use `scripts/run-benches.sh` with its current supported options; do not invoke `cargo bench --workspace`. Measure the operation claimed: in-memory write, buffered WAL append, durable commit, checkpoint, reopen, index rebuild and query are different workloads. Include small queries and mixed reads/writes, not only throughput-friendly large scans.

Use comparable fixtures, toolchain, native host and configuration. Record absolute timing, relative change, variability and memory where relevant. Serialize performance runs across agents. Do not build a new revision ledger; a concise before/after comparison with workload/configuration is sufficient. No fixed speedup or capacity target was validated in this review.

## Packaging and downstream checks

The product is an embedded Rust library. Inspect which workspace crates are actually public and publishable, package in their dependency order and build a small external consumer against packaged contents. Workspace path dependencies and unpublished internal versions must not hide missing files or an unusable dependency graph. A dry-run checks packaging without uploading, but it does not replace the dependency-order and consumer tests. [S14]

Attach useful GitHub-consumable artifacts through the authorized release workflow. Do not invent server binaries, a Python binding crate or wheels simply because other projects use Python. Bindings can be downstream work after the stable facade is usable.

## Report what happened

The handoff gives exact commands actually run, package/feature/codegen/runner selection, available pass/fail/skipped counts and unrun checks with reasons. Distinguish a focused test, a required PR gate and release evidence. This package contains planned checks; only PACKAGE-CHECK.md reports what was run while preparing the package itself.
