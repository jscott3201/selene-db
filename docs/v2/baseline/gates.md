# Archival gates and corpora

> Historical evidence for source `b8782bec34ff0b815b62711ac7e33cac09d8ea71` only.
> Not a 2.0 compatibility, signature, format-reader, alias, or migration contract.
> Benchmarks ran on an intentionally busy machine. They are non-green observations, not guards, comparisons, thresholds, or stable percentage baselines; issue #1137 / M08-PR06 owns future stable measurement.

Captured `2026-08-22T19:35:21Z` on `Darwin 25.6.0 arm64`.

Each command stands alone; non-passing results remain non-green. Raw redacted logs are ignored evidence.

| ID | Lane | Result | Exit | Seconds | Output SHA-256 | Command |
|---|---|---|---:|---:|---|---|
| `inventory-clone` | inventory | **passed** | 0 | 0.025 | `fadae4bab2ed77d90e84395914f043f372d47e5e89c487a1a4d304398707beb4` | `git clone --shared --no-checkout --local /Users/justin/Development/selene-db /var/folders/yr/g3z60l0d09g6_3vqc9ttccx80000gn/T/selene-v2-baseline-archive-sjzzw0a2/repository` |
| `inventory-checkout` | inventory | **passed** | 0 | 0.190 | `8c6fa7a32803424377d48c699b4cc5c3b6843d5e8b1ce709c5db5b7115374423` | `git checkout --detach b8782bec34ff0b815b62711ac7e33cac09d8ea71` |
| `inventory-rustdoc-core` | inventory | **passed** | 0 | 5.214 | `243a827495eec3cb1012e8b7d2b869d9e60ca824b3b41e37a3f66eee865ed62d` | `cargo +nightly-2026-08-15 rustdoc -Z unstable-options --locked -p selene-db-core --lib --output-format json` |
| `inventory-rustdoc-graph` | inventory | **passed** | 0 | 5.224 | `d777dae4062a1678b81a61d7e4900470f773287994a50cc4da0c5cbd8f2bc3be` | `cargo +nightly-2026-08-15 rustdoc -Z unstable-options --locked -p selene-db-graph --lib --output-format json` |
| `inventory-rustdoc-persist` | inventory | **passed** | 0 | 0.243 | `71e73f9925f4e2526e7e2bd667ab4a2b91bdc0acd0eebe7cf49c2ce92d8dab81` | `cargo +nightly-2026-08-15 rustdoc -Z unstable-options --locked -p selene-db-persist --lib --output-format json` |
| `inventory-rustdoc-algorithms` | inventory | **passed** | 0 | 1.703 | `27d29081a970a11191cc38674331f368b4042eb16d5c830178faf15045c126d5` | `cargo +nightly-2026-08-15 rustdoc -Z unstable-options --locked -p selene-db-algorithms --lib --output-format json` |
| `inventory-rustdoc-gql` | inventory | **passed** | 0 | 8.826 | `19993dcf29e2d78446efb5bcbf791b226dbf98aa4d4d6d4f027a28185a6626fa` | `cargo +nightly-2026-08-15 rustdoc -Z unstable-options --locked -p selene-db-gql --lib --output-format json` |
| `archive-fmt` | archive | **passed** | 0 | 1.427 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `cargo fmt --all --check` |
| `archive-check` | archive | **passed** | 0 | 19.014 | `b30e72017c481822bef512b70832b2079803ad0a3158abd342e76bf5c1b4ac70` | `cargo check --workspace --locked` |
| `archive-clippy` | archive | **passed** | 0 | 34.027 | `499be9a059d9e08044b736185686a608df647e5088579770cc5156d08764f8cd` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `archive-nextest` | archive | **passed** | 0 | 113.543 | `fc76d18bed34ae3aae8b820c2cdd38b3eb40e8f7630f2f12e12ecf4d7028bfbf` | `cargo nextest run --workspace --locked --all-features --profile default` |
| `archive-doctest` | archive | **passed** | 0 | 5.640 | `1ed04574d424511fa24acf365476e39dc71686388fead3952b1e04321d1118f0` | `cargo test --workspace --locked --all-features --doc` |
| `archive-doc` | archive | **passed** | 0 | 5.332 | `24aabb7f5c9a75a372f6b903cb9562ee8648ad34b2c43a5aa556f7624e5dfc67` | `cargo doc --workspace --no-deps --locked` |
| `archive-deny-bans` | archive | **passed** | 0 | 0.334 | `eba7f3001f2423b5c5fcdd45d2e210ba11daa9c0e73bba8072de1c60f30b5878` | `cargo deny check --exclude-dev bans` |
| `archive-deny-licenses-sources` | archive | **passed** | 0 | 0.316 | `a5aee31a15ab9c8780ae8f9e79160c98e6ec91e45093062c55a8352a96b3d53c` | `cargo deny check licenses sources` |
| `archive-audit` | archive | **passed** | 0 | 0.632 | `039d259515ec6390fe08f83e60879f7e3ef3316812b8d535bda652883f668c69` | `cargo audit -d /private/tmp/selene-advisory-db` |
| `archive-file-size` | archive | **passed** | 0 | 2.887 | `b241b1710f45112f117821de67c7fec72f2aa24c087dbf9745c0cae88baa45b6` | `bash .github/scripts/check-file-size.sh` |
| `archive-no-secrets` | archive | **passed** | 0 | 0.025 | `230d2198be82741fc5adc491328a219a545e75915988b11dd4dba4d8cd128b65` | `bash .github/scripts/check-no-secrets.sh` |
| `archive-thirdparty` | archive | **failed** | 1 | 12.361 | `c4ec50c3eeab741c4dd9efad9e52f55a2fffc40f68c6f8937028b40b0b052da7` | `bash .github/scripts/check-thirdparty-current.sh` |
|  |  | Reason |  |  |  | The archive did not pin cargo-about; sanctioned cargo-about 0.9.2 found immutable THIRDPARTY.md drift. M00-PR04 retains the failure without archive repair. |
| `archive-rowid` | archive | **passed** | 0 | 0.041 | `0472154f7b032817273d8dba1ac85a1fda10f6f0b7237e4d57b9c8654adb8afb` | `bash .github/scripts/check-no-rowid-arith.sh` |
| `archive-feature-errors` | archive | **passed** | 0 | 0.067 | `ca873fd5ffe98fe80677e1a3fb90da98b440a89b2690e7c15b2b20f10850d526` | `bash .github/scripts/check-no-version-locked-feature-error.sh` |
| `archive-bench-invocation` | archive | **passed** | 0 | 0.036 | `213b000f2047540965466b2c3b2b02b66f6d787c84202d9a0beaa9ee9058aa75` | `bash .github/scripts/check-bench-invocation.sh` |
| `archive-bench-doc` | archive | **passed** | 0 | 0.120 | `c7046a9668636991fde6f99e75da8aa9d1bb7b093c7346d606a15f2646bca5cc` | `bash .github/scripts/check-benchmarks-doc.sh .` |
| `archive-doc-constants` | archive | **passed** | 0 | 0.043 | `717b71a24e13e91233149459021c275ed1c99c66a06f07aff1cec03a3acab5f5` | `bash .github/scripts/check-doc-constants.sh` |
| `archive-mimalloc` | archive | **passed** | 0 | 0.013 | `11d40bf3e14b3ddef375b382b23723b4fed4f831d57faafd1735a5ad750686b8` | `bash .github/scripts/check-mimalloc-dev-dep.sh` |
| `benchmark-smoke` | benchmark | **passed** | 0 | 459.560 | `7d32079f2cdcf535ecead1cb51956bdc61794e8a97020c175266d4ab09862cce` | `scripts/run-benches.sh --smoke` |
| `benchmark-single-graph-read` | benchmark | **passed** | 0 | 142.809 | `2c8b7e035e760b1a095cfc3e0dfbfe0ae737830ba0e3eca2b940a12fe7e26289` | `scripts/run-benches.sh --profile full --bench single_graph --filter 'graph_(node_fetch\|label_index_lookup\|typed_index_point)'` |
| `benchmark-write-lifecycle` | benchmark | **passed** | 0 | 96.325 | `520be98d7640931a96378de38128c9f54a6bee9660afed61f01a40e0835f392d` | `scripts/run-benches.sh --profile full --bench write_txn_lifecycle --filter 'write_txn_lifecycle/(graph_clone\|create_only\|delete_only)'` |
| `benchmark-mixed-r60w40` | benchmark | **passed** | 0 | 43.808 | `3cfb28bd6201440f80e975d2aff520bb429d855267660bb58f145fc737b79f4f` | `scripts/run-benches.sh --profile full --bench graph_mixed_workload --filter graph_mixed_workload/point_read_update_r60w40` |
| `benchmark-write-lifecycle-repeat` | benchmark | **passed** | 0 | 328.371 | `afe27bcdef5cf3b8725a180b33d32aeb57fa71a70a855bf0b7129499481a1828` | `scripts/run-benches.sh --profile full --bench write_txn_lifecycle --filter 'write_txn_lifecycle/(graph_clone\|create_only\|delete_only)' --sample-size 100 --measurement-time 10` |
| `fuzz-gql-parse_gql-build` | fuzz | **passed** | 0 | 122.060 | `c16ace6d50c3f10e7911da52875e7f51c8b3e00604f1913b46dc75927de0f12b` | `cargo +nightly-2026-08-15 fuzz build parse_gql` |
| `fuzz-gql-parse_gql-run` | fuzz | **passed** | 0 | 11.700 | `096267a510d0e8e1ea6aee99da4adee4697a9518741d5368ea8f279d597508ac` | `cargo +nightly-2026-08-15 fuzz run parse_gql -- -max_total_time=10` |
| `fuzz-gql-parse_many_gql-build` | fuzz | **passed** | 0 | 0.789 | `4ea101167de926aaac0d63c492d3b0edf0a5f79d81cc230ba5cde9a2597160c6` | `cargo +nightly-2026-08-15 fuzz build parse_many_gql` |
| `fuzz-gql-parse_many_gql-run` | fuzz | **passed** | 0 | 11.288 | `06de5201ef047b435e8b2da9f24fff64e9d9f4316e68ba9b4254c87bf6655021` | `cargo +nightly-2026-08-15 fuzz run parse_many_gql -- -max_total_time=10` |
| `fuzz-gql-round_trip-build` | fuzz | **passed** | 0 | 0.672 | `326a661b3ec0ecd9a9e24ee0a60af1e5de2070c9721baa54d5b855483130b308` | `cargo +nightly-2026-08-15 fuzz build round_trip` |
| `fuzz-gql-round_trip-run` | fuzz | **passed** | 0 | 11.293 | `0da62a639060aa1ce794821b07143b4c4d64161e23975e83cfab3fe4e5e6ab62` | `cargo +nightly-2026-08-15 fuzz run round_trip -- -max_total_time=10` |
| `fuzz-persist-decode_manifest-build` | fuzz | **passed** | 0 | 20.918 | `81462b67937e6f50b0eedcbedeb22c24d1fada8ac28c32dc05153c0435e77959` | `cargo +nightly-2026-08-15 fuzz build decode_manifest` |
| `fuzz-persist-decode_manifest-run` | fuzz | **passed** | 0 | 11.243 | `9070c48d2596d891f661a34e1f2b35f021bc0778a52bd17bc5166921375a5194` | `cargo +nightly-2026-08-15 fuzz run decode_manifest -- -max_total_time=10` |
| `fuzz-persist-decode_wal-build` | fuzz | **passed** | 0 | 0.635 | `fe68704d702b503f5cf3f198bcfbcdff3aef2263784dbc2772be97023f5c00b0` | `cargo +nightly-2026-08-15 fuzz build decode_wal` |
| `fuzz-persist-decode_wal-run` | fuzz | **passed** | 0 | 11.222 | `e027646e19ce30c1753ed63c0f87c78964da56e8c9cbc64948e47ac5b68a3c8e` | `cargo +nightly-2026-08-15 fuzz run decode_wal -- -max_total_time=10` |
| `fuzz-persist-decode_audit-build` | fuzz | **passed** | 0 | 0.259 | `12964696dc9d2a01497c743d75bd36de3b7fc411253d9cc5a6337c7c9fc42243` | `cargo +nightly-2026-08-15 fuzz build decode_audit` |
| `fuzz-persist-decode_audit-run` | fuzz | **passed** | 0 | 11.233 | `4fa01a85a371048e4aa71d2a20e4cc0a6c9539d83549458505bab45a843f2232` | `cargo +nightly-2026-08-15 fuzz run decode_audit -- -max_total_time=10` |
| `fuzz-persist-decode_snapshot-build` | fuzz | **passed** | 0 | 0.263 | `12964696dc9d2a01497c743d75bd36de3b7fc411253d9cc5a6337c7c9fc42243` | `cargo +nightly-2026-08-15 fuzz build decode_snapshot` |
| `fuzz-persist-decode_snapshot-run` | fuzz | **passed** | 0 | 11.244 | `6d8618a5971c59f2f0176f9c37158419c265348fd731b8c0d6a9dd44edd36b5e` | `cargo +nightly-2026-08-15 fuzz run decode_snapshot -- -max_total_time=10` |

## Test and corpus identities

Nextest reported **4537 tests**.

| Corpus | Tracked path/prefix | Count | Unit |
|---|---|---:|---|
| parser-positive | `crates/selene-testing/corpus/positive/` | 134 | GQL files |
| parser-negative | `crates/selene-testing/corpus/negative/` | 36 | GQL files |
| planner-snapshots | `crates/selene-gql/tests/snapshots/plan_snapshot_corpus__` | 28 | snapshots |
| executor-snapshots | `crates/selene-gql/tests/snapshots/executor_snapshot_corpus__` | 22 | snapshots |
| algorithm-snapshots | `crates/selene-algorithms/tests/snapshots/algo_snapshot_corpus__` | 19 | snapshots |
| mutation-executor-test-files | `crates/selene-gql/tests/exec_pipeline_mutation` | 5 | Rust test files |
| mutation-plan-test-file | `crates/selene-gql/tests/plan_mutation.rs` | 1 | Rust test files |

## Fuzz targets

| Crate | Target | Source | Tracked seeds |
|---|---|---|---:|
| `selene-gql` | `parse_gql` | `crates/selene-gql/fuzz/fuzz_targets/parse_gql.rs` | 0 |
| `selene-gql` | `parse_many_gql` | `crates/selene-gql/fuzz/fuzz_targets/parse_many_gql.rs` | 0 |
| `selene-gql` | `round_trip` | `crates/selene-gql/fuzz/fuzz_targets/round_trip.rs` | 0 |
| `selene-persist` | `decode_manifest` | `crates/selene-persist/fuzz/fuzz_targets/decode_manifest.rs` | 0 |
| `selene-persist` | `decode_wal` | `crates/selene-persist/fuzz/fuzz_targets/decode_wal.rs` | 0 |
| `selene-persist` | `decode_audit` | `crates/selene-persist/fuzz/fuzz_targets/decode_audit.rs` | 0 |
| `selene-persist` | `decode_snapshot` | `crates/selene-persist/fuzz/fuzz_targets/decode_snapshot.rs` | 0 |

## Known ignored and slow tests

### Ignored

- `helper_process` in `crates/selene-persist/src/manifest_lock/tests.rs`: ignored helper invoked by separate_process_contends_on_the_same_lock_file
- `local_spec_corpus_snapshots_match` in `crates/selene-gql/tests/plan_snapshot_corpus.rs`: local-only specification mirror; run manually with --ignored

### Slow

- `parsing_hostile_fold_then_dropping_is_safe` in `crates/selene-gql/tests/parser_expr_depth.rs`: nextest slow-timeout override: 48 seconds, terminate after five periods

## Observation notes

- Archive execution used an isolated local clone with a detached checkout and its own .git directory.
- Cargo network access was disabled for every canonical command; service and embedding variables were removed from child environments.
- git diff --check is intentionally absent from the archive lane and belongs to the current harness worktree gate.
- The archive did not pin cargo-about. The current sanctioned cargo-about 0.9.2 reports attribution drift; immutable archive output is retained as failed evidence and is not repaired by M00-PR04.
- All four persistence fuzz targets built and completed their short runs on macOS despite the archive persistence fuzz README saying Linux-only; no Linux run is claimed.
- No tracked fuzz seed corpus exists for the seven required targets; cargo-fuzz starts from its built-in seed for short runs.
- The machine was intentionally busy. Benchmark numbers are non-green observations only; no guard, threshold, optimization claim, or future percentage comparison derives from them. Issue #1137 / M08-PR06 owns stable measurement and guard selection.
- Initial absolute benchmark runs have no comparison p-value. Criterion repeat p-values are internal same-tree run-to-run signals and are not product regression evidence.
- The required mixed filter matched the intended non-WAL rows and their WAL-suffixed companions; both are disclosed.
- A coefficient of variation above 0.25 triggered one sanctioned 100-sample, 10-second-measurement write repeat. Higher fidelity did not stabilize the rows.
