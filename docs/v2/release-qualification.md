# F06-PR02 — local alpha qualification

Captured 2026-09-13 on `development` at
`17d0984dce07f39cd727eb2c032e1100e58f7977` plus the uncommitted F06-PR02 package/fuzz
work and F06-QUAL-04 repair. These are worktree results, **not a clean exact-head
release attestation**. [Release notes](release-notes-alpha.md) are a draft;
publication, tags, releases and issue closure remain unauthorized.

## Native and artifact boundary

Native host: macOS 27.0 (26A5425a), arm64; rustc 1.97.1 (`8bab26f4f`, LLVM 22.1.6),
nextest 0.9.143; cargo nightly 1.99.0 (`3efb1f477`). **Linux unavailable, not passed.** No QEMU, cross-compilation,
alternate CPU qualification or power-loss certification was performed.

Passed:

```sh
bash .github/scripts/check-package-release.test.sh
bash .github/scripts/check-package-release.sh
cargo package --workspace --exclude selene-db-testing --locked --allow-dirty
bash .github/scripts/check-package-release.sh --archives target/package
bash scripts/smoke-packaged-crates.sh
```

`--allow-dirty` permits this authorized uncommitted handoff; it does not disable
Cargo's verification. All eight `.crate` files were built from packaged contents.
Dependency order is core, profile, catalog, persist, graph, algorithms, GQL,
facade (package names in `.github/scripts/public-crates.txt`). There are 1,580
packaged files across the eight archives. The non-publishable testing helper may
appear in Cargo's temporary staging directories but has no distributable archive.

The checker compares root legal texts with all 32 per-crate copies and extracted
archive copies. Its negative cases reject missing/stale texts and duplicate
inventory entries. Normalized package metadata contains versioned registry
requirements, no path-only dependencies. The external consumer has an independent
lock/target and patches only the eight extracted candidate directories, never
workspace source. It passed six tests with default features and six with all
engine features: catalog smoke, mixed-edge/path identity, constraints/no partial
publication, negative requests, native retrieval, transaction/checkpoint/reopen.
Fresh consumer resolution emitted two `wide::swizzle_relaxed` deprecation warnings;
the locked workspace clippy gate passed without suppressions or dependency edits.

### Artifact audit

Inspected Cargo-verified extracted source, normalized manifests, module routing
and facade exports, in addition to the archive legal checks:

- Persist exports control, logical frames/snapshots/streams and retained directory
  capabilities. The private `legacy_probe.rs` reads at most eight header bytes and
  rejects retired SLDB/SLSN/SLMF/SLAU artifacts; it does not decode their payloads.
  No old WAL/snapshot/manifest/audit decoder or migration module is shipped.
- GQL's `runtime/plan_runner.rs`, `runtime/pattern.rs` and
  `runtime/pipeline/mod.rs` enter physical batch execution unconditionally. The
  scalar evaluator and materialized binding tables remain intentional kernels and
  result/barrier carriers, not an old production executor or row fallback.
- The facade's exports and compile-fail doctests exclude GraphHandle, SharedGraph,
  Mutator, WalWriter, RowIndex, core graph-type bridges and lower execution types.
  Intentional scalar/type/element-ID re-exports remain, not stale public bridges.
- Lower graph schema/default adapters still use historical `Legacy`/`V1` names.
  They are current in-memory descriptor/serde boundaries, not format-1 store
  decoders or facade bridges; absence of every historical word is not claimed.
  Fuzz workspaces/corpora/artifacts are not packaged. No extension pack, server or
  binding product was invented.

## F06-QUAL-04 parser repair

Before editing, the exact saved-input command from the brief reproduced exit 70
after 22 seconds with `-timeout=20`. `cargo +nightly fuzz tmin parse_gql
artifacts/parse_gql/timeout-9623e2a3c0e4f4d4fd4c7fa140275d3cc03a5f23 -- -timeout=20
-max_total_time=120` reduced 57 bytes to a 14-byte timeout reproducer: thirteen
`{` bytes followed by `0x1e`. The minimizer's final 13-byte *slow-unit* output did
not time out; the regression embeds the last verified 14-byte timeout, not that
misleading final output path. Twelve openers plus a trailing byte took about
10–11 seconds; eight took 155 ms under ASan. Minimality is bounded by this native
timeout/minimization run, not a platform-independent timing theorem.

Hypothesis, supported by source and probes: `nested_query` re-enters
`query_specification`, whose composite/chained/plain alternatives retry the same
pipeline, multiplying work per bare wrapper. The general depth-64 guard does not
bound this adequately; the existing tighter list guard counts only `[`.

The fix extends the pre-pest scan with a fixed-size stack of seven active
brace-to-brace wrappers (eight consecutive bare levels). Comments do not hide
transitions; quoted braces do not count; only matching brace closes release the
pressure, so tokens inside a still-open wrapper do not erase it. Violations are
`ComplexityLimitExceeded` / `5GQL1`. Quote helpers were moved unchanged out of the
near-cap guard file before extension. No pest global setting, grammar, persisted
format, runtime semantics or timeout was changed.

The existing artifact test embeds the minimized bytes and asserts the error/status
plus its existing 250 ms tripwire. Boundary tests include comments, balanced
over-budget input, interposed VALUE bodies, eight admitted levels, many sibling
wrappers, deep records and quoted braces. All 64 focused tests passed, including
the existing 45-level EXISTS/deep-fold stack test:

```sh
cargo nextest run -p selene-db-gql --locked --all-features --profile ci --test parser_dos_artifacts --test dos_guard --test parser_expr_depth --test parse_many_dos --test parser_case_depth
```

Original and minimized post-fix replays each completed in 0 ms. From
`crates/selene-gql/fuzz`, the required campaign passed:

```sh
cargo +nightly fuzz run parse_gql -- -max_total_time=300 -timeout=20 -max_len=65536
```

261,386 executions in 306 seconds, seed 3851063608, no crash/timeout. Nightly
reported an existing `Atomic::fetch_update` deprecation; the pinned stable gate
does not. All corpus/artifact files remain ignored and outside the patch.

### Current persistence decoder campaign

From `crates/selene-persist/fuzz`, serially ran `cargo +nightly fuzz run <target>
-- -max_total_time=20 -timeout=20 -max_len=65536` on every release target. All passed
in 21 seconds each:

| Target | Executions |
|---|---:|
| decode_control | 337,053 |
| decode_manifest | 358,724 |
| decode_wal | 112,172 |
| decode_snapshot | 484,601 |
| decode_logical | 144,807 |
| decode_logical_value | 769,667 |
| decode_logical_snapshot | 117,764 |

These exercise current format-2 raw/structured/integrity-repaired boundaries.
The retired audit target is deleted rather than reviving its removed decoder.
This bounded release campaign is not the longer nightly soak or exhaustive fuzzing.

## Serialized performance and memory guards

No task-owned build/fuzzer/second benchmark overlapped measurements. Ordinary
desktop scheduling was not exclusively controlled. Default mimalloc, unchanged
fixtures, bench opt-level 3/thin LTO/one codegen unit, no CPU flag changes:

```sh
scripts/run-benches.sh --profile quick --bench read_write_guard --bench facade_read_write
scripts/run-benches.sh --profile full --bench read_write_guard --bench facade_read_write --sample-size 30 --measurement-time 4
scripts/run-benches.sh --profile full --bench facade_read_write --sample-size 30 --measurement-time 4
```

Quick completed 16 graph + two facade rows. Full completed 48 graph rows at
10k/50k/100k plus four facade rows at 1k/10k, followed by the unchanged-binary
facade stability recheck. Full used 30 samples, 100 ms warmup, four-second target
measurement. Compare with the F05-PR07 native control in `BENCHMARKS.md`, not the
old canonical typed-*miss* rows:

| 100k row | Canonical (95% CI) | Sparse (95% CI) |
|---|---:|---:|
| fetch | 23.689 [23.549–23.891] ns | 22.425 [22.341–22.513] ns |
| label | 10.510 [10.453–10.574] ns | 39.776 [39.733–39.830] ns |
| edge label | 7.7565 [7.7011–7.8186] ns | 41.399 [41.188–41.578] ns |
| typed hit | 14.734 [14.640–14.852] ns | 13.639 [13.450–13.838] ns |
| checked x1 | 7.1874 [7.1460–7.2242] µs | 7.2983 [7.2825–7.3204] µs |
| checked x8 | 56.067 [55.748–56.587] µs | 58.454 [58.375–58.568] µs |
| clone/drop | 24.449 [24.414–24.510] µs | 9.9069 [9.8936–9.9281] µs |
| mixed r60/w40 | 291.20 [277.50–308.45] µs | 240.74 [233.23–249.80] µs |

Compared with the documented final MapM control, canonical label is +5.8%, edge
label +2.8%, clone +1.0%; canonical/sparse checked-x8 are −7.3%/−10.1% and mixed
−5.8%/−1.0%. These are cross-run observations, not attributed optimizations or a
numeric CI gate. Typed-hit costs are below the corrected F05 restored-A costs.

The first full facade pass rose to 76.273/76.409 µs reads and 131.95/164.82 µs
updates (1k/10k), versus quick 56.720/109.51 µs at 1k. With **no source or binary
change**, the same full command rechecked at 57.240/56.923 µs reads and
115.29/152.00 µs updates; CIs were [57.146–57.343], [56.781–57.116],
[114.99–115.67], [151.64–152.37] µs. The transient slowdown is retained as a failed
stability observation, not hidden as an engine regression or speedup. Recheck
reads are −6.8%/−8.9% and updates −0.5%/−0.9% versus F05's final native control.
No persistent material regression is demonstrated by this guard set.

Fresh sparse-process RSS (representative triplicate, bytes; not exact live heap):

| Scale | Built | 16 clones | 16 retained versions |
|---|---:|---:|---:|
| 10k | 108,806,144 | 108,822,528 | 114,851,840 |
| 50k | 406,601,728 | 406,650,880 | 408,616,960 |
| 100k | 819,625,984 | 819,625,984 | 822,804,480 |

100k triplicates varied by 16,384 bytes; 10k/50k were identical. Values remain
within 0.1% of the documented F05 MapM RSS. Construction temporaries and retained
allocator pages remain included; zero observed clone delta is not allocation-free.

## Gates and publication readiness

Fresh workspace checks passed: formatting; locked default/all-feature check;
all-target clippy with warnings denied; nextest CI and default profiles each
**4,859 passed, five skipped**, no retries used; **33 doctests**; no-deps rustdoc; default/all-feature
production dependency bans; licenses/sources; live `cargo audit` (1,243 advisories,
292 dependencies). The developer-path audit fetch failed because the existing
directory was non-empty; its `--no-fetch` variant passed independently. No cached
audit result substitutes for the successful live default-path audit.

The all-feature run includes durable outcomes, corruption/failpoints, native
process-kill/reopen, reader/writer/retention, constraints, batch/path/native and
public API/profile cases together. The five ignored entries are three child
process helpers (driven by parent tests, not selected directly), the opt-in
`overhead` timing comparison and the local-only spec-mirror invariant test.
No optional external embedding service was invoked.

Profile generation and conformance-doc checks passed, as did the baseline harness
(18 tests), plan validation (6 milestones/35 items/7 issues) and its **16/16**
negative-contract suite. File-size, secrets, third-party freshness, row-ID,
feature-error, benchmark invocation/registry, documentation constants, allocator
placement, repository-home and diff-whitespace scripts passed. Release-tag
*validation tests* and benchmark/doc/allocator/runner regression scripts passed;
they create no release tag and do not publish.

Exact workspace and policy commands (all passed except the disclosed audit fetch):

```sh
cargo fmt --all --check
cargo check --workspace --locked
cargo check --workspace --locked --all-features
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --all-features --profile ci
cargo nextest run --workspace --locked --all-features --profile default
cargo test --workspace --locked --all-features --doc
cargo doc --workspace --no-deps --locked
cargo deny --exclude-dev check bans
cargo deny --all-features --exclude-dev check bans
cargo deny check licenses sources
cargo audit -d /private/tmp/selene-advisory-db
cargo audit
cargo audit --no-fetch -d /private/tmp/selene-advisory-db
cargo run --locked -p selene-db-profile --bin selene-profile -- --check
cargo run --locked -p selene-db-testing --bin selene-conformance -- docs --check --root .
python3 -B scripts/v2_baseline.test.py
python3 -B .github/scripts/check-v2-plan.py --root .
python3 -B .github/scripts/check-v2-plan.test.py
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
bash .github/scripts/check-thirdparty-current.sh
bash .github/scripts/check-no-rowid-arith.sh
bash .github/scripts/check-no-version-locked-feature-error.sh
bash .github/scripts/check-bench-invocation.sh
bash .github/scripts/check-benchmarks-doc.sh .
bash .github/scripts/check-doc-constants.sh
bash .github/scripts/check-mimalloc-dev-dep.sh
bash .github/scripts/check-repository-home.sh .
bash .github/scripts/check-release-tag.test.sh
bash .github/scripts/check-bench-invocation.test.sh
bash .github/scripts/check-benchmarks-doc.test.sh
bash .github/scripts/check-doc-constants.test.sh
bash .github/scripts/check-mimalloc-dev-dep.test.sh
bash scripts/run-benches.test.sh
git diff --check
```

The audit path above is the actual developer-path invocation, not a portable
prerequisite; use the live default `cargo audit` on another host.

**PUBLICATION-READINESS:** local native macOS package/consumer and bounded release
qualification evidence is available for review. Publication is **pending**, not
authorized or claimed complete. Linux and clean exact-head claim/hosted release
checks remain unrun here; the dirty worktree cannot satisfy the clean-revision
wrapper. The delivery owner must bind final evidence to the reviewed commit and
run the real native lane before any separately authorized publication. The formal
selected-profile claim must remain denied under the narrowed alpha decision.
