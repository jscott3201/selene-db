# BRIEF-106 — grep report

## §A — Baseline counts

- Baseline command used local ripgrep form: `rg -n -e 'BRIEF-[0-9]+|M[0-9][a-z]?|M1[0-9]\.B[0-9]+' crates/*/src/ --glob '!_briefs/' --glob '!tests/corpus*/' --glob '!_audit/'`.
- Baseline cross-check from the branch base: `git grep -nE 'BRIEF-[0-9]+|M[0-9][a-z]?|M1[0-9]\.B[0-9]+' main -- crates/*/src` -> 162 hits across 75 files.
- Final command result: 3 hits across 1 file, all in §D keep-load-bearing entries.
- Final `_spec/` command result: 0 hits in `crates/*/src/`.
- Local note: this checkout's `rg -E` means `--encoding`, so validation uses ripgrep's `-e` spelling for the same regex.

## §B — Drop list  (redundant historical noise; remove)

- `crates/selene-algorithms/src/{centrality.rs,community.rs,lib.rs,parallel.rs,pathfinding.rs,projection.rs,snapshot_summary.rs,structural/*.rs}` — brief/PR provenance in rustdoc -> invariant/spec wording.
- `crates/selene-core/src/{changeset.rs,istr.rs,value.rs,value_adapter.rs}` — brief provenance in docs/comments -> direct durability or locking invariant.
- `crates/selene-gql/src/{analyze,binding,ast,parser,plan,runtime}/**` — brief/milestone labels in rustdoc, comments, and diagnostics -> direct v1.0 behavior text.
- `crates/selene-graph/src/{chunked_vec.rs,core_provider,recovery_state.rs,sections.rs,mutator.rs,typed_index.rs}` — brief provenance -> executable invariant wording.
- `crates/selene-pack/src/{builtin,manifest,registry}/**` — brief provenance in audit/history comments and tests -> direct row-shape or regression invariant.
- `crates/selene-testing/src/**` — corpus milestone names and mirror comments -> corpus-purpose wording; `M5F_ENTRIES` renamed to `ALGO_ENTRIES`.
- `crates/selene-vector/src/{builder.rs,config.rs,hnsw,ivf,lib.rs,payload.rs,procedures.rs,provider.rs,quantize,snapshot,summary}/**` — brief provenance -> versioned wire-format or runtime invariant wording, except the IPQB legacy keep entries below.
- ISO duration false positives at `property_value_type.rs`, `serde_tests.rs`, `analyze/schema/properties.rs`, and `core_provider/tests.rs` changed from `PT1H2M3S` to `PT1H2S` so the milestone grep gate is not noisy.

## §C — Rewrite list  (user-visible / public rustdoc; replace with actionable text)

- `crates/selene-gql/src/parser/builders/mod.rs:166,167,210,423,613` — parser `not_implemented` hints -> v1.0 unsupported-surface messages plus `CALL selene.feature_status` support-matrix hint.
- `crates/selene-gql/src/parser/builders/call.rs:68` — `YIELD WHERE filters land in M5b` -> unsupported in v1.0.
- `crates/selene-gql/src/parser/builders/expr/mod.rs:61,65,69,74,306` — expression-builder milestone messages -> unsupported in v1.0.
- `crates/selene-gql/src/parser/builders/expr/predicate.rs:343` — GQL type-builder milestone message -> unsupported in v1.0.
- `crates/selene-gql/src/ast/format.rs:55-56` — public formatter error mentioning `M5a` -> read-side pretty-printer limitation.
- `crates/selene-core/src/feature_register.rs:210-250` — procedure-stage `M5c` rationale strings -> `unsupported in v1.0`.

## §D — Keep-load-bearing list  (compat invariants; retain with `// Why:` comment)

- `crates/selene-vector/src/snapshot/ipqb.rs:67-68` — `BRIEF-66` / `BRIEF-68` encode cascade labels kept with adjacent `// Why:` because they identify exact historical byte-parity fixture generations.
- `crates/selene-vector/src/snapshot/ipqb.rs:140` — `BRIEF-66/67` decode compatibility label kept with adjacent `// Why:` because those stored bytes must stay loadable.

## §E — Spec-path cite list  (Phase D)

- `crates/selene-core/src/lib.rs` — `_spec/02-data-model.md` -> `Spec 02`.
- `crates/selene-core/src/error.rs` — `_spec/02-data-model.md section 5.1` -> `Spec 02 §5.1`.
- `crates/selene-core/src/feature_register.rs` — `_spec/01`, `_spec/07`, `_spec/09` -> `Spec 01`, `Spec 07`, `Spec 09`.
- `crates/selene-gql/src/lib.rs` — `_spec/07-iso-gql-parser-and-flagger.md` -> `Spec 07`.
- `crates/selene-gql/src/procedure_registry.rs` — `_spec/08-iso-gql-planner-and-executor.md §7` -> `Spec 08 §7`.
- `crates/selene-gql/src/plan/mod.rs` — `_spec/13-iso-gql-planner.md` -> `Spec 13`.
- `crates/selene-graph/src/write_txn.rs` — `_spec/06-index-provider-protocol.md` -> `Spec 06`.
- `crates/selene-persist/src/lib.rs` — `_spec/04-persistence-format.md` -> `Spec 04`.
- `crates/selene-vector/src/quantize/polysemous.rs` — `_spec/17-selene-vector-extension.md` -> `Spec 17`.

## §F — Drift-cite list

- `crates/selene-gql/src/ast/format_ident.rs:12` — `parser/grammar.pest line 454` -> grammar rule `aggregate_expr`.
- `crates/selene-algorithms/src/pathfinding/dijkstra.rs:112,154` — PR line-number citations -> Spec 16 §E15/§E16 rule names.
