# Selene DB 2.0 work items M00–M04

<!-- Generated from plan.json; do not edit by hand. -->

The machine plan carries additional design, path, documentation, and benchmark metadata for each contract.

<a id="m00-pr01"></a>
## M00-PR01 — Declare 1.x End-of-Life and Establish the 2.0 Line

- **Owner:** M00
- **State:** Merged
- **Risk / size:** High / M
- **Dependencies:** None
- **Issues:** None
- **Commit scope:** `release`

Make the support and version break executable: archive the final 1.x source snapshot, move the active workspace to 2.0 alpha, and remove every repository statement that implies future 1.x maintenance.

### Scope

- Verify the working branch contains `b8782bec34ff0b815b62711ac7e33cac09d8ea71` or explicitly record and review any newer base before editing.
- Bump all workspace/package versions from 1.x to `2.0.0-alpha.1` and update path dependency version constraints atomically.
- Add a prominent 1.x EOL statement: no fixes, security patches, compatibility maintenance, new 1.x releases, or persisted-format migration support.
- Document the owner-only archival actions for protected branch `archive/1.x-final` and annotated tag `archive-v1-eol-2026-08-21` at `b8782bec34ff0b815b62711ac7e33cac09d8ea71`.
- Update release workflow comments and checks so non-semver archive tags do not publish crates and 2.0 pre-release tags are handled intentionally.
- Delete or rewrite staged “1.5.0” language in release notes, plans, and agent instructions; do not silently call unreleased work a 1.5 release.

### Non-goals

- No engine architecture or behavior change beyond version/support/release controls.
- No attempt to publish 1.5.0, create a 1.x patch branch, or build a migration tool.
- No source deletion from the archive snapshot; archival Git objects preserve it.
- No 2.0 API design beyond links to the finalized decision document.

### Acceptance evidence

- `cargo metadata --locked --format-version 1` shows `2.0.0-alpha.1` for all Selene workspace packages and no 1.x path dependency constraints.
- A repository-wide search finds no forward-looking promise for 1.5.0 or 1.x maintenance outside historical changelog entries clearly marked historical.
- README and CHANGELOG state the EOL policy in the first relevant release/support section.
- Release/tag workflow tests prove `archive-v1-eol-*` is not a publish trigger and intended 2.0 tags still are.
- The PR description contains the exact post-merge owner commands for archive branch/tag creation and their verification commands.
- No crate API or file format compatibility shim is added.

### Tests and gates

- Run `cargo metadata --locked --format-version 1` and inspect package and dependency versions.
- Run the repository metadata/release scripts affected by version changes.
- Add or update shell tests for tag classification where the release workflow uses pattern logic.
- Run `cargo check --workspace --locked` to catch mixed path-version constraints.
- Run `git grep -nE "1\.5\.0|maintain(ed|ing)? 1\.x|1\.x support"` and classify every remaining match.

### Review focus

- No accidental crates.io publication path.
- Every package/version pin changed together.
- EOL wording has no exception that recreates a maintenance obligation.
- Archive actions point to the exact reviewed SHA.

### Stop conditions

- The source SHA differs and contains unreviewed changes.
- A current deployment owner requires 1.x fixes or a migration path.
- The archive tag would trigger existing release automation.
- Version bump exposes unrelated compile failures that cannot be isolated.

### Bridge and deletion

- No bridge. The PR intentionally removes compatibility promises.
- After merge, the repository owner creates and protects the archive branch/tag; implementers do not perform Git, GitHub, settings, release, or archive mutations.

<a id="m00-pr02"></a>
## M00-PR02 — Commit the Finalized 2.0 Architecture and Milestone Contract

- **Owner:** M00
- **State:** Merged
- **Risk / size:** Medium / M
- **Dependencies:** M00-PR01
- **Issues:** None
- **Commit scope:** `docs`

Install the approved architecture decisions, master milestone map, PR review protocol, and issue mapping as tracked repository documentation that future agents must follow.

### Scope

- Add the finalized decision record covering product boundary, facade, catalog scope, conformance policy, contexts, identity, edge directionality, compiler, executor, constraints/indexes, persistence, extensions, and release criteria.
- Add the milestone/PR package or a repository-native copy of its canonical documents under tracked `docs/v2/` paths; underscore working directories remain untracked.
- Add the PASS/FIX/REPLAN review protocol with separate implementer, orchestrator, and independent read-only reviewer roles.
- Map every current open issue to its owning 2.0 PR and state that issues are closed only by the owning implementation PR.
- Link 2.0 docs from README and AGENTS without turning AGENTS into a fast-moving PR ledger.
- Record source snapshot, assumptions, research boundaries, and the fact that the initial review did not independently execute the Rust suite.

### Non-goals

- No engine code changes.
- No GitHub milestone/issue mutation in this PR.
- No copying of the licensed ISO PDF or substantial normative text into the repository.
- No re-opening finalized architecture choices without an explicit REPLAN decision.

### Acceptance evidence

- All relative links in the 2.0 documentation resolve in a link-check script.
- The documented milestone and PR counts match the canonical plan data.
- The seven current open issues each map to exactly one owning PR.
- AGENTS clearly separates implementer edit/test work from orchestrator Git, PR, review-comment, and eligible authorized merge mutations and includes the required handoff fields.
- No tracked document directs future work into a gitignored underscore directory.
- ISO references use clause/feature/implementation-defined IDs and paraphrase rather than reproducing the standard.

### Tests and gates

- Run Markdown link validation over `docs/v2`.
- Run docs-only repository gates: formatting where applicable, file-size, secret scan, doc constants/registry scripts, and `git diff --check`.
- Verify generated tracker JSON schema if included.
- Inspect repository search results for contradictory merge/support instructions.

### Review focus

- Decision text matches the finalized package exactly.
- No hidden optionality around 1.x support and no ambiguity in the corrected role or merge-eligibility model.
- PR slices are implementation-sized rather than broad epics.
- Tracked versus local working-document rules are coherent.

### Stop conditions

- A finalized decision conflicts with current source facts discovered during installation.
- The plan would require committing licensed ISO material.
- The generated package and Markdown disagree on identifiers or dependencies.

### Bridge and deletion

- This document set supersedes the earlier high-level review roadmap for implementation sequencing.
- Keep the original review notes under a research/history subsection; do not delete the evidence behind the plan.

<a id="m00-pr03"></a>
## M00-PR03 — Enforce the 2.0 PR, Review, and CI Operating Model

- **Owner:** M00
- **State:** Merged
- **Risk / size:** High / M
- **Dependencies:** M00-PR02
- **Issues:** None
- **Commit scope:** `ci`

Turn the 2.0 operating model into repository automation: practical Rust checks on every non-draft development PR, deterministic plan validation, complete handoff evidence, and exact-head merge eligibility.

### Scope

- Add a development-PR Rust compile/test lane that is bounded enough for routine work but cannot accept syntactically valid, uncompiled architecture changes.
- Validate milestone/PR identifiers, dependency references, issue ownership, and generated tracker freshness.
- Install a PR template requiring plan ID, scope, deviations, tests, benchmarks, bridge/deletion obligations, and handoff summary.
- Document non-draft PR handling, the independent reviewer pair, exact-head merge eligibility, and the post-merge branch-protection settings action.
- Keep full cross-platform, audit, fuzz, and exhaustive gates on the main/release path while defining risk-triggered gates for each PR.
- Replace the blanket agent merge prohibition with the corrected implementer/orchestrator separation and explicit authorization conditions.

### Non-goals

- No self-hosted runner or new CI service.
- No requirement to run every expensive fuzz/benchmark job on every PR.
- No automated assistant approval bot.
- No change to engine behavior.

### Acceptance evidence

- A deliberate Rust compile error in a disposable isolated copy fails the exact `cargo check --workspace --locked --all-features` command invoked by development CI.
- A broken plan dependency, duplicate PR ID, missing Markdown file, and stale tracker each fail the plan validator fixture tests.
- PR template records the complete handoff, role separation, exact-head review state, and merge-eligibility confirmations without claiming technical enforcement.
- Release workflow still covers Linux/macOS, nextest, doctests, deny/audit, attribution, parser fuzz, and persist decoder fuzz.
- Required check names are stable and documented for branch protection configuration.
- CI does not use secrets or untrusted PR text unsafely in shell commands.

### Tests and gates

- Add positive and negative fixture tests for the plan validator.
- Run workflow syntax validation or actionlint when available.
- Run repository shell-script tests.
- Use a disposable copy outside the repository to prove the exact cargo-check lane detects a deliberate Rust compile failure.
- Run `git diff --check` and existing secret scan.

### Review focus

- The routine lane actually compiles/tests code.
- No unsafe workflow interpolation.
- Role separation and every merge-eligibility condition are explicit and internally consistent.
- Plan validator is deterministic and does not depend on network access.

### Stop conditions

- Development CI cost grows beyond the agreed routine budget without a narrower valid gate.
- A required job can be skipped or deadlocked on an eligible non-draft PR.
- Workflow changes accidentally weaken release gates.

### Bridge and deletion

- Existing cheap gates remain until the new lane is green; delete redundant checks only after proving equivalent coverage.
- No temporary bypass labels for architecture PRs.

<a id="m00-pr04"></a>
## M00-PR04 — Capture the Executable 1.x Baseline and 2.0 Deletion Inventory

- **Owner:** M00
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M00-PR03
- **Issues:** None
- **Commit scope:** `test`

Run and record the complete source, test, benchmark, public-API, persistence-format, and corpus baseline at the archival SHA so later PR reviews can distinguish intended breaks from accidental regressions.

### Scope

- Check out or compare against `b8782bec34ff0b815b62711ac7e33cac09d8ea71` and run the complete local gate documented in AGENTS with commands, tool versions, durations, and outcomes.
- Capture public symbols and examples for every published 1.x crate as a deletion/inventory aid, not as a compatibility contract.
- Inventory WAL, snapshot, MANIFEST, audit, package, procedure, feature-register, and generated-doc version identities.
- Run sanctioned benchmark smoke plus the read/write rows implicated by #1137 and record per-section timestamps and hardware context.
- Record corpus counts, fuzz seed locations, mutation suites, test counts, and any known flaky/slow tests.
- Create a machine-readable baseline manifest keyed by source SHA and command, with hashes for generated reports.

### Non-goals

- No attempt to make failing baseline tests green inside this PR unless the failure is caused by the baseline harness itself.
- No compatibility test promise for 2.0.
- No benchmark optimization.
- No copying crates.io artifacts into the repository.

### Acceptance evidence

- All full-gate commands have recorded exit codes and logs/summaries; any failure has an issue or explicit prerequisite decision.
- The baseline manifest identifies Rust/tool versions, OS/architecture, repository SHA, and benchmark hardware context.
- Every existing public crate and persisted artifact appears in the deletion/inventory documents.
- Read and write hot-path benchmarks include isolated current measurements and do not rely on the stale June baseline.
- The baseline script is idempotent, keeps secrets out of logs, and writes generated files to a controlled path.
- No baseline artifact is presented as independently rerun by the planning assistant; execution evidence comes from this PR.

### Tests and gates

- Run the full AGENTS local gate.
- Run `scripts/run-benches.sh --smoke` and focused full-profile rows for node fetch, label lookup, typed index point, clone, create, delete, and mixed workload.
- Compile and short-run parser and persistence fuzz targets.
- Validate baseline JSON against its schema and verify report hashes.
- Run the baseline script twice and compare deterministic metadata sections.

### Review focus

- Evidence is executable and tied to the archival SHA.
- Failures are visible, not edited out.
- No compatibility promise is accidentally created.
- Benchmark provenance is section-level and balanced across reads/writes.

### Stop conditions

- The pinned SHA cannot reproduce or the branch has changed without an owner decision.
- A baseline failure indicates data-loss/corruption risk requiring an immediate prerequisite PR.
- Benchmark environment is too noisy to support the recorded decision.

### Bridge and deletion

- The public API inventory exists solely to ensure deliberate deletion/replacement; it is removed or archived after 2.0 GA.
- This PR closes no product issue; it creates evidence for all later reviews.

<a id="m01-pr01"></a>
## M01-PR01 — Introduce the Canonical `selene-profile` Registry

- **Owner:** M01
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M00-PR02
- **Issues:** None
- **Commit scope:** `profile`

Create the leaf profile crate and schema that become the sole source of truth for GQL feature, implementation choice, extension, and evidence metadata.

### Scope

- Add `crates/selene-profile` as a dependency-light leaf crate with no engine-crate dependencies and `publish = false` during alpha.
- Define stable records for feature IDs, claim state, clause anchors, implied feature IDs, implementation-defined IDs, implementation-dependent notes, extension IDs, evidence IDs, and applicability expressions.
- Add checked-in JSON profile source plus a JSON Schema; use only identifiers, short names, and paraphrased decisions.
- Implement deterministic loading, validation, sorting, hashing, and code/document generation APIs.
- Move or adapt the current feature-register ownership so runtime consumers can depend on the profile without circular dependencies.
- Add explicit claim states such as `unsupported`, `implemented_unclaimed`, `claimed_pending_evidence`, and `claimed` rather than one Boolean.

### Non-goals

- No full import of Table 10 or Annex B yet.
- No change to current parser/runtime behavior except routing through a compatibility adapter.
- No normative ISO prose in source data.
- No dynamic network fetch or build-time dependency on the licensed PDF.

### Acceptance evidence

- `selene-profile` builds independently and has no dependency path to core/graph/gql.
- Invalid IDs, duplicate IDs, unknown references, cyclic generation inputs, unstable ordering, and missing required fields have focused failures.
- Running the generator twice produces byte-identical outputs and the same profile hash.
- Current feature-status callers can read through the adapter without hand-maintained duplicate data.
- Schema/source files contain no long copied ISO passages.
- A profile format/version bump is required for incompatible registry-schema changes.

### Tests and gates

- Unit tests for schema validation and identifier/reference checks.
- Golden tests for deterministic JSON/Rust/Markdown generation.
- Dependency graph check that `selene-profile` is a leaf.
- Round-trip test from source JSON to typed records and canonical JSON.
- Existing feature-register tests through the adapter.

### Review focus

- No circular dependency or duplicated truth.
- Claim states are expressive enough to separate implementation from evidence.
- Generator determinism and error messages.
- Licensed text is not reproduced.

### Stop conditions

- The proposed crate dependency direction creates a core/profile cycle.
- The schema cannot represent applicability or evidence without embedding arbitrary executable code.
- Current runtime consumers require a breaking move too large for this PR.

### Bridge and deletion

- Keep a thin `selene-core` feature-register re-export/adapter until M01-PR04.
- Do not add new features to the old hand-maintained register after this merges.

<a id="m01-pr02"></a>
## M01-PR02 — Encode Feature Taxonomy and Transitive Implication Closure

- **Owner:** M01
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M01-PR01
- **Issues:** None
- **Commit scope:** `profile`

Populate the canonical feature inventory and Table 10 relationships, calculate transitive closure, and make contradictory claims impossible to generate or merge.

### Scope

- Encode the feature IDs and short names relevant to the current and target Selene profile, plus all features reachable through their implication closure.
- Encode direct Table 10 implication edges with clause/table provenance and generator-version metadata.
- Implement cycle detection, transitive closure, minimal missing-dependency diagnostics, and a topological implementation order where possible.
- Reclassify current high-confidence conflicts: GE04/GV60, GE05/GV61, GG01/GG02/GC04, GS04/GS05–GS08, GV66/GV67/GV65, and undirected-edge claims.
- Generate a claim matrix showing direct versus transitive requirements and evidence state.
- Add a CI check that no `claimed` feature has an unclaimed or evidence-incomplete implied feature.

### Non-goals

- No implementation of missing graph/reference/session features in this PR.
- No attempt to encode every normative rule; that is M01-PR05.
- No claim of completeness beyond the imported feature/closure inventory and its reviewed provenance.

### Acceptance evidence

- All listed current implication conflicts produce failing fixtures before reclassification and pass only after the source state is corrected.
- Closure tests cover chains, diamonds, cycles, duplicate edges, unknown IDs, and minimal missing-path diagnostics.
- Generated matrix identifies whether each dependency is direct or transitive and links its evidence state.
- CI fails when a claimed feature dependency is downgraded or omitted.
- The target profile cannot be marked release-claimable while any closure member is pending evidence.
- A reviewer can trace every encoded edge to a clause/table reference in the licensed source without copied normative text.

### Tests and gates

- Property tests comparing closure with a simple reference graph algorithm.
- Golden tests for the generated implication matrix and diagnostics.
- Negative fixtures for each known current conflict.
- Mutation tests around edge removal, claim-state comparisons, and transitive dependency handling.
- Profile-schema validation in CI.

### Review focus

- Table 10 edges and transitive closure are accurate.
- No feature is silently dropped to make the gate green.
- Diagnostics show actionable dependency paths.
- Claim state and implementation state remain distinct.

### Stop conditions

- Feature IDs or relationships cannot be verified from the licensed source.
- The imported subset is insufficient to close dependencies for the target profile.
- A current runtime feature name is being conflated with a different ISO feature.

### Bridge and deletion

- Current public feature claims are downgraded to the generated truthful state immediately.
- Later implementation PRs update profile/evidence records as part of their acceptance criteria.

<a id="m01-pr03"></a>
## M01-PR03 — Build the Exact Annex B Implementation-Defined Profile

- **Owner:** M01
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M01-PR01
- **Issues:** None
- **Commit scope:** `profile`

Replace the partial/misaligned implementation-defined ledger with exact IDs, applicability, chosen values, rationale, and evidence hooks for the selected profile.

### Scope

- Audit and encode every Annex B implementation-defined ID applicable to the selected profile or current extension surface.
- Correct IA001 to the result-declared-type exposure decision and relocate floating-point policies to their actual IDs or extension records.
- Lock initial values for catalog depth, default collation, Unicode profile, preferred names, source repertoire, time zone, session parameters, match mode, cardinality limits, result-type exposure, diagnostics, procedure provisioning, and transaction mechanisms.
- Represent non-applicability with an explicit applicability expression and rationale rather than omission.
- Generate Rust constants/types, release documentation, and test vectors from the registry.
- Fail CI on duplicate/missing applicable IDs, unknown evidence references, or placeholder decisions in a release-claimable profile.

### Non-goals

- No full implementation of every selected behavior; evidence can remain pending.
- No bundled authentication or locale service.
- No assertion that Annex B is normative; it is used as the collected index for implementation-defined occurrences.

### Acceptance evidence

- IA001 has the correct decision and a test of result descriptor exposure.
- Every applicable selected-profile implementation-defined occurrence resolves to exactly one registry record.
- Non-applicable entries explain which absent feature or product boundary makes them non-applicable.
- Generated docs contain no “TBD” in a release-claimable state.
- Limits are enforced by boundary tests or marked pending with owning PR IDs.
- A repository search finds no second hand-authored Annex B value table.

### Tests and gates

- Schema and completeness tests against the curated ID occurrence list.
- Golden generation tests.
- Boundary tests for already-implemented choices such as result type exposure, default match mode, and cardinality limits.
- Negative tests for unknown IDs and type-invalid values.
- Mutation tests for applicability evaluation and completeness gating.

### Review focus

- ID-to-decision mappings, especially IA001.
- Typed choices and stability/upgrade effects.
- Completeness and non-applicability logic.
- No hidden product scope such as auth/server.

### Stop conditions

- An ID occurrence or meaning cannot be confidently verified.
- A chosen value would prematurely constrain an unimplemented architecture without evidence.
- Unicode/collation versioning cannot be made persistence-safe with the current plan.

### Bridge and deletion

- Delete the old `feature_register/annex_b.rs` table or reduce it to generated re-exports in M01-PR04.
- Every later PR that implements a pending decision must add its evidence ID.

<a id="m01-pr04"></a>
## M01-PR04 — Generate Flagger, Feature Status, and Documentation from the Profile

- **Owner:** M01
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M01-PR02, M01-PR03
- **Issues:** None
- **Commit scope:** `gql`

Cut over all parser/analyzer/runtime and documentation consumers to generated profile data and delete the old hand-maintained feature and Annex B registries.

### Scope

- Replace runtime feature/Annex B static tables with generated `selene-profile` artifacts.
- Make the GQL Flagger query profile capability/evidence states and distinguish unsupported syntax, extension syntax, and implemented-but-unclaimed behavior.
- Update `selene.feature_status()` or its 2.0 successor to expose profile hash, feature state, direct/implied status, and evidence summary without leaking internal paths.
- Generate feature, implementation-defined, extension, and claim-summary documentation from one command.
- Delete compatibility adapters and prevent direct edits to generated output.
- Update plan-cache keys to include profile hash/version when analyzer behavior depends on enabled features.

### Non-goals

- No new syntax or feature implementation.
- No release claim transition to green.
- No dynamic per-request arbitrary feature toggling unless already part of the approved profile model.

### Acceptance evidence

- Repository search finds no duplicate hand-maintained feature or Annex B list.
- A profile source edit updates all generated runtime/docs artifacts through one deterministic command.
- Flagger positive/negative tests consume generated capability states and emit the correct feature IDs.
- Feature status reports direct and implied dependencies and the exact profile hash.
- Plan cache invalidates when the effective profile hash changes.
- CI fails on stale generated artifacts.

### Tests and gates

- Run all parser/analyzer/flagger/feature-status tests.
- Golden test generated docs and runtime tables.
- Cache-key tests across profile changes.
- Negative tests for extension/unsupported/implemented-unclaimed distinctions.
- Mutation tests around claim-state branching.

### Review focus

- Old truth sources are actually deleted.
- Flagger semantics align with claim states.
- Profile hash participates in cache correctness.
- Generated docs accurately show blockers.

### Stop conditions

- A runtime behavior relies on feature metadata not representable in the canonical schema.
- Deleting the old register would require unrelated feature implementation.
- Plan cache cannot safely include profile identity without broader redesign; isolate and REPLAN.

### Bridge and deletion

- This PR deletes the M01-PR01 compatibility adapter.
- No new direct use of generated internal arrays outside the profile API.

<a id="m01-pr05"></a>
## M01-PR05 — Add the Normative Rule/Evidence Harness and Claim Gate

- **Owner:** M01
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M01-PR04
- **Issues:** None
- **Commit scope:** `test`

Create the rule-to-code-to-test evidence registry and a release gate that refuses conformance claims until every mandatory and selected feature rule has complete executable evidence.

### Scope

- Define rule/evidence records for clause/rule identity, applicability, feature dependencies, implementation symbols, expected statuses, positive tests, negative tests, model/differential tests, persistence/crash tests, mutation tests, and current disposition.
- Seed the registry with the target core profile and every known blocker rather than pretending completion.
- Build a test harness that can select evidence by rule, feature, clause, milestone, or release claim.
- Generate a traceability matrix and machine-readable claim manifest from actual test results and source/profile hashes.
- Add a release gate that permits “ISO-aligned” publication with disclosed gaps but blocks stronger claim text unless evidence is complete.
- Add source-reference checks that detect stale symbols/test names after refactors.

### Non-goals

- No attempt to finish all rule evidence in this PR.
- No external certification claim.
- No embedding of full normative rule prose.
- No replacement of ordinary unit/integration test organization.

### Acceptance evidence

- The harness can answer “what evidence supports feature X?” and “what blocks the target profile?” deterministically.
- At least one positive, negative, status, and pending fixture demonstrates each evidence state.
- Deleting or renaming a referenced test/symbol fails the source-reference check.
- The release gate blocks a deliberately overstated claim and permits a truthful gap-disclosing profile artifact.
- Generated matrix links every seeded rule to feature/profile applicability and owning milestone/PR.
- Claim manifest includes repository SHA, profile hash, test command, result hash, and generation tool version.

### Tests and gates

- Self-tests for evidence selection, stale references, pending/blocker logic, and claim wording gates.
- Run the seeded conformance suite and generate a known-incomplete manifest.
- Mutation tests for gate state transitions.
- Golden tests for traceability matrix and claim manifest.
- Release workflow dry run in non-publishing mode.

### Review focus

- The gate measures evidence, not feature labels.
- Expected statuses/types/side effects are represented.
- Stale references fail loudly.
- Claim wording cannot be manually stronger than the manifest.

### Stop conditions

- Rule inventory scope cannot be verified for the selected profile.
- The gate would require brittle parsing of Rust source instead of explicit evidence registration.
- Release wording policy remains disputed.

### Bridge and deletion

- The seeded registry is intentionally incomplete; every later PR must update its owned evidence records.
- M10-PR05 is the only PR allowed to transition the selected release claim to complete.

<a id="m02-pr01"></a>
## M02-PR01 — Add the `selene-catalog` and Stable `selene-db` Facade Skeleton

- **Owner:** M02
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M00-PR04, M01-PR01
- **Issues:** None
- **Commit scope:** `facade`

Create the new crate boundaries and a minimal `Database`/`DatabaseBuilder`/`Session` facade over the existing engine without yet implementing full catalog DDL.

### Scope

- Add `selene-catalog` depending only on `selene-core`/`selene-profile` and `selene-db` depending on the engine layers.
- Define `Database`, `DatabaseBuilder`, `DatabaseConfig`, `Session`, `OpenMode`, and top-level error/outcome re-exports with minimal stable semantics.
- Wrap one existing graph behind an internal bootstrap catalog so the facade can execute a current query end to end.
- Define the lower-crate stability policy in rustdoc and mark facade exports intentionally.
- Add compile-fail/privacy tests that prevent public exposure of internal `SharedGraph`, row indices, mutators, or persistence writers from the stable API.
- Move the primary quickstart to the facade while clearly marking the bootstrap catalog as temporary.

### Non-goals

- No multi-schema/named graph management yet.
- No catalog persistence.
- No removal of every old API until the vertical catalog slice exists.
- No server, connection pool, async runtime, or authentication service.

### Acceptance evidence

- A minimal program depends only on `selene-db`, creates an in-memory database, opens a session, writes, and queries through the facade.
- `Session` has no borrowed `SharedGraph` lifetime and can be moved independently while the database remains alive.
- Facade rustdoc does not expose internal row, graph, WAL, or provider concrete types in public signatures.
- The workspace dependency graph remains acyclic and `selene-catalog` does not depend on graph/gql/persist.
- Existing tests still run through temporary internal adapters.
- The quickstart no longer instructs new users to assemble six crates directly.

### Tests and gates

- Facade smoke/integration tests for in-memory create/write/query/drop.
- Compile-fail or public API snapshot tests for internal-type leakage.
- Dependency graph/cargo metadata assertion.
- Existing session/query tests through compatibility adapter.
- Rustdoc and doctest build for `selene-db`.

### Review focus

- The facade is a real ownership boundary, not a re-export bag.
- No lifetime or concrete-type leakage.
- Crate dependency direction.
- Temporary bootstrap is explicitly scheduled for removal.

### Stop conditions

- A required facade signature exposes `SharedGraph` or row-space types.
- Crate layering creates a cycle.
- The bootstrap adapter requires duplicating mutation semantics.

### Bridge and deletion

- A private single-graph bootstrap adapter may call current APIs.
- Delete the bootstrap catalog in M02-PR05 after real named graphs are operational.

<a id="m02-pr02"></a>
## M02-PR02 — Implement Persistent Catalog IDs, Canonical Names, and Flat Schema Descriptors

- **Owner:** M02
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M02-PR01, M01-PR03
- **Issues:** None
- **Commit scope:** `catalog`

Define the catalog object identity and immutable descriptor model for one synthetic root directory containing multiple schemas and primary catalog objects.

### Scope

- Add opaque stable IDs for catalog, directory, schema, graph, graph type, binding table, procedure, index, and constraint objects with kind-safe wrappers.
- Implement canonical identifier/name services using the selected Unicode/collation/canonical-form profile.
- Define immutable descriptors with ID, canonical name, display spelling where retained, owner schema/parent, object kind, generation, creation metadata, and kind-specific payload.
- Implement one synthetic root directory with schemas only and reject child-directory creation at the catalog service boundary.
- Move reusable schema/graph-type metadata from `selene-core` only where it avoids duplication and does not introduce cycles.
- Add catalog snapshot/generation types suitable for lock-free analysis reads.

### Non-goals

- No graph storage registry or DDL execution.
- No access-control policy beyond opaque owner/principal fields.
- No nested directories, aliases, search paths, or cross-catalog references.
- No persisted encoding until M09.

### Acceptance evidence

- Schema dictionaries enforce canonical-name uniqueness and distinguish namespaces only where the profile permits.
- Unicode-equivalent/case/collation edge cases have explicit canonicalization tests and generated profile references.
- Catalog snapshots are immutable and can be read without holding a mutation lock.
- No public API uses raw integers/UUIDs as interchangeable catalog object IDs.
- Child-directory operations fail with a structured unsupported-profile diagnostic.
- Descriptor equality, identity, and generation semantics are documented and tested.

### Tests and gates

- Property tests for ID kind separation, serialization-ready round trips, and canonical-name dictionary behavior.
- Unicode corpus tests tied to the selected profile version.
- Snapshot immutability and generation tests.
- Negative tests for duplicate canonical names and child directories.
- Mutation tests around name equality and object-kind checks.

### Review focus

- Canonical-name correctness and Unicode versioning.
- Kind-safe identity.
- Immutable snapshot boundary.
- No dependency cycle or premature persistence representation.

### Stop conditions

- Canonicalization/collation choice is not finalized or reproducible.
- Descriptor payload requires graph storage concrete types.
- A flat catalog cannot support an already-required use case; REPLAN rather than sneak in directories.

### Bridge and deletion

- Current graph type/schema structs may be converted at the facade boundary temporarily.
- Remove duplicate schema definitions when M02-PR05 completes migration.

<a id="m02-pr03"></a>
## M02-PR03 — Add Catalog-Owned Named Graph and Graph-Type Lifecycle in Rust

- **Owner:** M02
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M02-PR02
- **Issues:** None
- **Commit scope:** `catalog`

Introduce one transactional catalog service for creating, resolving, opening, and dropping schemas, named graphs, and graph types through stable IDs and descriptors.

### Scope

- Implement catalog read snapshots and a serialized catalog mutation transaction/funnel.
- Add Rust APIs for create/drop/resolve/list schema, graph, and graph type with IF EXISTS/IF NOT EXISTS semantics represented explicitly.
- Add a graph instance registry keyed by stable `GraphId`/catalog object ID and owned by `DatabaseInner`.
- Bind optional constraining graph-type descriptors to graphs and validate lifecycle dependencies.
- Define drop dependency/restrict behavior; defer CASCADE unless already in the selected extension profile and explicitly scoped.
- Publish catalog and graph-registry changes atomically in memory, with persistence hooks reserved but not yet durable.

### Non-goals

- No GQL grammar/DDL execution yet.
- No cross-database graph references.
- No graph rename/alter beyond minimum lifecycle needed.
- No persistence or crash recovery.

### Acceptance evidence

- Multiple schemas and named graphs coexist in one database and resolve by absolute catalog path.
- Duplicate names, missing parents, referenced graph-type drops, and graph/schema drops with contents produce structured outcomes.
- A failed mutation publishes neither descriptor nor graph instance.
- A concurrent reader sees either the old or new complete catalog snapshot, never a partial registry state.
- Graph type binding is visible through descriptors and enforced on subsequent mutations.
- Rust and future GQL DDL can call the same catalog service without duplicating validation.

### Tests and gates

- Catalog lifecycle integration tests across multiple schemas/graphs/types.
- Atomic publication/failure injection tests.
- Reader snapshot concurrency tests.
- Dependency/drop restriction tests.
- Reference invalidation tests for dropped objects.
- Property tests for mutation sequences against a simple catalog model.

### Review focus

- One mutation funnel and atomic publication.
- Descriptor/storage separation.
- Dependency validation.
- No duplicated DDL semantics.

### Stop conditions

- Catalog and graph registry cannot be published atomically without redesigning ownership.
- Graph type binding depends on mutable schema structures not yet moved.
- Drop semantics are ambiguous with active sessions/transactions.

### Bridge and deletion

- Graph storage may still use current `SharedGraph` internally, keyed by catalog ID.
- Direct construction remains private/deprecated until M02-PR05 deletes it.

<a id="m02-pr04"></a>
## M02-PR04 — Implement GQL Catalog DDL over the Catalog Service

- **Owner:** M02
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M02-PR03, M01-PR04
- **Issues:** None
- **Commit scope:** `gql`

Route CREATE/DROP SCHEMA, GRAPH, and GRAPH TYPE syntax through semantic resolution and the same catalog transaction service used by Rust APIs.

### Scope

- Audit/complete strict GQL forms for create/drop schema, graph, and graph type within the selected profile.
- Resolve absolute/relative object references against session/current working schema and canonical catalog paths.
- Lower DDL to catalog service commands with explicit IF EXISTS/IF NOT EXISTS, restrict, and outcome semantics.
- Return structured GQLSTATUS and omitted-result outcomes through the facade.
- Update feature/evidence records for graph management and dependent open/closed graph claims only to implemented/pending-evidence states.
- Add introspection tests proving GQL and Rust lifecycle APIs produce identical descriptors.

### Non-goals

- No nested directory syntax.
- No CREATE PROCEDURE/BINDING TABLE.
- No ALTER/RENAME/CASCADE beyond explicitly existing and selected extension behavior.
- No persisted DDL until M09.

### Acceptance evidence

- Positive/negative syntax and semantic tests cover schema/graph/type lifecycle and relative/absolute references.
- Rust API and GQL DDL create byte/field-equivalent descriptors under the same profile/context.
- Duplicate/missing/dependency failures return exact expected GQLSTATUS records.
- Feature implication conflicts involving GC04/GG01/GG02 move only to truthful states backed by tests.
- DDL cannot bypass the catalog mutation funnel or graph-type dependency checks.
- No direct graph mutation occurs before catalog command validation succeeds.

### Tests and gates

- Parser corpus and source-span snapshots.
- Analyzer resolution tests across schemas/session defaults.
- Runtime DDL integration tests and Rust/GQL equivalence tests.
- Expected GQLSTATUS tests for duplicate/missing/dependency cases.
- Conformance evidence updates and gate run.
- Parser fuzz short run due grammar changes.

### Review focus

- Strict syntax and correct resolution.
- One catalog service path.
- Status/outcome precision.
- No overclaim of closed/open graph conformance.

### Stop conditions

- Current grammar donor behavior conflicts with the licensed standard and cannot be resolved in this slice.
- DDL needs the full M03 transaction model to be correct; split/reorder rather than add an ad hoc transaction.
- Relative reference semantics remain ambiguous.

### Bridge and deletion

- Temporary catalog auto-commit adapter may remain only until M03-PR04.
- No second DDL-specific catalog store.

<a id="m02-pr05"></a>
## M02-PR05 — Cut Over Public Construction and Sessions to the Catalog-First Facade

- **Owner:** M02
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M02-PR04
- **Issues:** None
- **Commit scope:** `facade`

Remove the bootstrap single-graph ownership path, migrate examples/tests to `Database`, and internalize or delete 1.x direct graph/session entry points.

### Scope

- Make `DatabaseBuilder` and database-owned `Session` the sole documented and stability-promised entry points.
- Replace bootstrap graph selection with catalog schema/graph resolution.
- Migrate integration tests, examples, benchmarks, and native procedure setup to named graph contexts.
- Make direct `SharedGraph` construction/session execution private, crate-internal, or explicitly unstable advanced API as required by internal crates.
- Remove public 1.x compatibility aliases, deprecation shims, and quickstart imports rather than carrying them through alpha.
- Update public API inventory to mark replaced/removed symbols and verify no facade signature leaks them.

### Non-goals

- No session/request semantic completion; M03 owns it.
- No persistence open/recovery.
- No native feature reintegration beyond keeping existing tests compiling internally.
- No attempt to preserve source compatibility.

### Acceptance evidence

- All top-level examples and doctests compile using only `selene-db` imports for ordinary use.
- A public API snapshot contains no stable `Session<'g>`, raw `SharedGraph`, `Mutator`, `RowIndex`, `WalWriter`, or provider concrete types.
- Multiple named graphs can be created and queried through separate/current graph selections.
- The bootstrap adapter from M02-PR01 is deleted.
- Repository docs do not instruct users to combine layer crates for normal operation.
- All existing engine tests either use internal test helpers or the facade without public compatibility aliases.

### Tests and gates

- Public API snapshot/compile-fail checks.
- All examples/doctests.
- Multi-graph facade integration tests.
- Workspace tests for migrated fixtures.
- Repository search for removed 1.x entry-point names in public docs.

### Review focus

- Old public ownership root is truly gone.
- No facade leakage or compatibility feature flag.
- Examples exercise named graphs.
- Bridge deletion is complete.

### Stop conditions

- A native subsystem cannot compile without public access and needs a planned internal trait/handle first.
- The facade cannot select a graph without duplicating session state that M03 will replace.
- Removal breaks repository tooling in a way requiring an explicit prerequisite slice.

### Bridge and deletion

- Delete M02 bootstrap and 1.x public adapters.
- Internal crate-private direct graph helpers may remain for tests but are not re-exported or documented.

<a id="m03-pr01"></a>
## M03-PR01 — Implement `SessionContext`, Principal, and Profile Defaults

- **Owner:** M03
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M02-PR05, M01-PR03
- **Issues:** None
- **Commit scope:** `session`

Replace ad hoc session fields with one explicit context carrying authorization, home/current schema and graph, time zone, parameters, transaction slot, request slot, profile identity, and lifecycle state.

### Scope

- Define immutable/controlled `SessionContext` fields matching the selected embedded profile: authorization ID, principal, time-zone displacement, session schema, session graph, session parameters, active transaction, active request, and termination flag.
- Add `PrincipalProvider`/`AuthorizationPolicy` traits with a deterministic local allow-all default and no network/auth storage implementation.
- Resolve optional home schema/home graph at session creation and define behavior when they are absent or have been dropped.
- Initialize defaults from the canonical profile and expose typed session inspection through the facade.
- Make context mutation private to session commands/state-machine methods.
- Include catalog/profile generation dependencies required for plan and reference validity.

### Non-goals

- No full privilege language or user administration.
- No request/execution context yet.
- No transaction implementation beyond a typed vacant/occupied slot.
- No thread-safe concurrent use of one mutable session unless explicitly designed later.

### Acceptance evidence

- Session creation produces exactly the documented profile defaults and resolves home objects consistently.
- Invalid authorization IDs, missing home objects, or dropped references produce structured session errors/diagnostics.
- No public caller can mutate context fields without using validated session methods.
- Session inspection never exposes internal locks/graph instances.
- Profile/time-zone/default parameter values are linked to Annex B evidence records.
- Plan/cache dependency metadata includes session schema/graph/profile identity where relevant.

### Tests and gates

- Session default creation tests with and without home schema/graph.
- Custom provider/policy test doubles for allow/deny/error paths.
- Dropped-reference invalidation tests.
- Profile default boundary tests.
- Compile-time trait assertions for intended Send/Sync behavior.
- Mutation tests around context initialization and authorization checks.

### Review focus

- Clear distinction between auth hook and auth service.
- Context mutation ownership.
- Reference/generation validity.
- Profile defaults are generated, not duplicated.

### Stop conditions

- A context field requires M03 request/transaction behavior to define safely; represent it as a typed slot, do not implement ad hoc semantics.
- The auth trait leaks internal catalog/graph concrete types.
- Session thread-safety expectations are unresolved.

### Bridge and deletion

- Old session fields may be populated from SessionContext temporarily.
- Delete the old session struct layout by M03-PR05.

<a id="m03-pr02"></a>
## M03-PR02 — Implement `RequestContext`, Typed Parameters, Timestamp, and Request Outcome

- **Owner:** M03
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M03-PR01
- **Issues:** None
- **Commit scope:** `session`

Give each execution a fresh request context that merges parameters deterministically, fixes the request timestamp, owns the execution stack, and returns one structured outcome.

### Scope

- Define `Request`, `RequestParams`, `GeneralParameter`, `RequestContext`, `RequestTimestamp`, and `RequestOutcome` facade/runtime types.
- Merge session parameters and request parameters with request-name shadowing and explicit declared value types.
- Validate parameter names, duplicates, values, reference validity, and assignability before execution begins.
- Capture one zoned request timestamp lazily or eagerly according to the locked profile and reuse it for all current-time functions in the request.
- Associate exactly one active request context with a session during execution and clear it on every success/failure/unwind path.
- Map parser/analyzer failures into failed request outcomes with structured diagnostics rather than bypassing the request model.

### Non-goals

- No full execution-context stack implementation.
- No async/cancellation API changes beyond carrying existing cancellation state.
- No transport serialization format.
- No arbitrary session/request concurrency.

### Acceptance evidence

- Request parameters shadow session parameters exactly by canonical name and do not mutate the session dictionary.
- Duplicate, ill-typed, invalid-reference, and unknown parameter uses return expected diagnostics before side effects.
- Multiple current-time calls in one request observe the same timestamp; consecutive requests may differ.
- Parse/analyze/runtime failures all return the same top-level request outcome type.
- The session cannot begin a second request while one is active and is reusable after a completed failure.
- Request cleanup is covered under panic/failure injection.

### Tests and gates

- Parameter merge/typing/property tests.
- Timestamp consistency tests with a fake clock.
- Parse/analyze/runtime failure outcome tests.
- Active-request reentrancy and cleanup tests.
- Reference invalidation tests for graph/table/node/edge parameters as implemented.
- Mutation tests around shadowing and cleanup.

### Review focus

- Uniform failure/outcome path.
- Timestamp semantics.
- Parameter assignability and reference validity.
- No session mutation leakage.

### Stop conditions

- Reference parameter semantics require unimplemented reference types; scope to implemented kinds and mark evidence pending.
- Cleanup cannot be guaranteed without restructuring the execution entry point.
- Parameter typing duplicates compiler type logic rather than calling it.

### Bridge and deletion

- Current parameter maps may be adapted into RequestParams until M05 type work.
- Delete direct executor parameter arguments once all calls use RequestContext.

<a id="m03-pr03"></a>
## M03-PR03 — Implement Execution Context Stack, Binding Tables, and Structured Outcomes

- **Owner:** M03
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M03-PR02
- **Issues:** None
- **Commit scope:** `runtime`

Model root and child execution contexts with working record/table, declared result descriptors, status bundles, and deterministic push/pop semantics independent of the current executor implementation.

### Scope

- Define root/child `ExecutionContext`, `ExecutionStack`, immutable `Record`, binding-table descriptor/table interfaces, and `ExecutionOutcome` variants.
- Represent successful regular result, successful omitted result, no-data/warning statuses, and failed outcomes with primary/additional/nested status objects.
- Track declared result type separately from runtime value/table and enforce the IA001 exposure policy at the facade boundary.
- Implement push/pop/child-copy/amend rules needed by current procedure, statement, and expression execution.
- Guarantee working record and working table field-name disjointness at construction/mutation boundaries.
- Adapt the existing executor to use these contexts without yet changing its row-at-a-time operators.

### Non-goals

- No batch representation; M06 owns it.
- No complete semantic type redesign; M05 will replace temporary type adapters.
- No path-value compact representation.
- No status localization service.

### Acceptance evidence

- Root/child context construction matches documented invariants and stack cleanup occurs on every exit path.
- Field-name overlap is rejected or normalized according to exact operation semantics rather than silently overwritten.
- Outcome precedence and successful omitted/no-data/failed distinctions have focused tests.
- Declared type exposure follows the generated IA001 decision and never exposes empty type for omitted results.
- Existing supported query/procedure outcomes can be represented without lossy conversion.
- No public facade result requires importing runtime internal context types.

### Tests and gates

- Context construction/push/pop/property tests.
- Outcome/status precedence and nested-cause tests.
- Working-record/table disjointness tests.
- Existing executor differential tests before/after adapter.
- Panic/failure stack cleanup tests.
- Mutation tests for outcome variant and precedence branches.

### Review focus

- Outcome semantics and status precision.
- Immutable table/record contract.
- No batch- or type-system overreach.
- Facade/internal separation.

### Stop conditions

- Existing executor mutates tables in place in a way that cannot be adapted safely.
- Status precedence differs across code paths and needs a prerequisite consolidation.
- Declared type cannot be preserved without M05 type descriptors; introduce an opaque temporary descriptor, not strings.

### Bridge and deletion

- Temporary adapters from existing BindingTable/StatementOutput are allowed.
- M06 replaces table physical storage while preserving this semantic/outcome API.

<a id="m03-pr04"></a>
## M03-PR04 — Implement the Serializable Transaction State Machine and Demarcation

- **Owner:** M03
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M03-PR03, M02-PR04
- **Issues:** None
- **Commit scope:** `transaction`

Unify implicit auto-commit and explicit START/COMMIT/ROLLBACK under one session-owned transaction object with precise active, failed, committing, rolled-back, committed, and indeterminate states.

### Scope

- Define `Transaction`, `TransactionId`, characteristics/access mode, state enum, catalog snapshot/base generation, graph snapshots/deltas, and outcome classification.
- Route START/COMMIT/ROLLBACK commands and implicit procedure auto-start through one state machine.
- Enforce at most one active transaction per session, one request within one transaction, and serial statement visibility within the transaction.
- Stage catalog and graph changes without publication until successful commit; failed procedure execution attempts rollback.
- Define allowed mixing of catalog- and data-modifying work for the initial profile and return exact errors for unsupported mixes.
- Provide a durability-independent commit interface that M09 will implement with append/flush/publish phases.

### Non-goals

- No weaker isolation levels or MVCC.
- No external/encompassing transaction manager integration.
- No durable WAL implementation change yet.
- No distributed/multi-database transaction.

### Acceptance evidence

- Implicit one-statement and explicit multi-request transactions share tests and produce equivalent committed state where semantics align.
- Successor statements observe successful predecessor changes inside the same transaction; other sessions do not before publication.
- Failed statements/procedures trigger the required rollback attempt and leave no catalog/graph publication.
- Active-state, duplicate START, COMMIT/ROLLBACK without active transaction, read-only writes, and unsupported mixing return expected GQLSTATUS.
- Catalog and graph deltas are committed or rolled back together in memory.
- Commit authority interface has explicit canceled, committed, and indeterminate outcomes for M09.

### Tests and gates

- State-machine model/property tests over command sequences.
- Multi-request explicit transaction integration tests.
- Cross-session visibility tests.
- Failure injection in validation/catalog/graph preparation/publication abstraction.
- Read-only and mixing policy tests with exact statuses.
- Mutation tests for transition guards.

### Review focus

- One state machine and atomic catalog/data staging.
- Precise rollback versus indeterminate classification.
- No hidden publication in mutators.
- Serializable semantics.

### Stop conditions

- Current graph commit cannot be prevented from publishing before catalog coordination.
- Catalog/data atomic staging requires an ownership change larger than this PR; split a prerequisite.
- Commit outcome vocabulary cannot map to current error/status model without ambiguity.

### Bridge and deletion

- Use an in-memory commit authority adapter until M09.
- Delete catalog DDL auto-commit bridge from M02-PR04.

<a id="m03-pr05"></a>
## M03-PR05 — Complete Session Commands, Working Schema/Graph, Close, and Multi-Request Semantics

- **Owner:** M03
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M03-PR04
- **Issues:** None
- **Commit scope:** `session`

Finish the context control plane with SET/RESET/CLOSE behavior, scoped working schema/graph resolution, session lifecycle cleanup, and generation-safe plan reuse.

### Scope

- Implement selected profile forms for session schema, graph, time zone, parameter, reset, reset-all, and close commands with implication-closed feature states.
- Resolve current working schema references and working graph sites during annotation with nested AT SCHEMA/USE GRAPH scopes.
- Define close behavior with active transaction rollback and termination flag handling.
- Invalidate or re-analyze cached plans when session defaults, catalog generations, graph type generations, procedure generations, or profile hash change.
- Delete remaining old session layout/adapters and direct executor session mutation.
- Add multi-request scenario tests covering state persistence, reset, failure, drop/recreate, and close.

### Non-goals

- No connection/network lifecycle.
- No nested directories/search path list.
- No arbitrary simultaneous requests in one session.
- No full procedure catalog yet.

### Acceptance evidence

- Every claimed session feature passes direct and implied-feature tests; unsupported forms are flagged truthfully.
- SET/RESET failures do not partially mutate context.
- Nested working schema/graph scopes resolve deterministically and restore outer scope.
- Dropping/recreating an object with the same name does not let an old plan bind to the new object ID accidentally.
- Close with/without active transaction has exact outcomes and makes the session unusable afterward.
- All old session fields/adapters are removed.

### Tests and gates

- Positive/negative parser/analyzer/runtime session command corpus.
- Multi-request state-machine scenarios.
- Nested scope resolution snapshots.
- Plan-cache invalidation tests across context/catalog/profile changes.
- Close/rollback failure injection tests.
- Conformance implication/evidence gate updates.

### Review focus

- Implication closure for RESET ALL.
- Object ID/generation-safe plan cache.
- Atomic session mutations.
- Old session bridge deletion.

### Stop conditions

- A selected session feature implies unimplemented reference-value behavior; withdraw claim or split implementation, never ignore implication.
- Plan cache cannot express all dependencies.
- Close semantics conflict with active commit authority behavior.

### Bridge and deletion

- Delete all old Session layout and direct executor context paths.
- M05 may replace annotation internals but not the public context semantics.

<a id="m04-pr01"></a>
## M04-PR01 — Formalize Stable Element Identity, Reference Handles, and Generation Tokens

- **Owner:** M04
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M02-PR03, M03-PR01
- **Issues:** None
- **Commit scope:** `identity`

Create the identity contract that separates persistent object/element IDs, session/request reference values, private storage rows, and graph/catalog generations.

### Scope

- Define distinct stable ID types for catalog objects, graphs, nodes, and edges; define private `NodeRow`/`EdgeRow` wrappers inside graph storage.
- Define `GraphGeneration`, `CatalogGeneration`, and optional store epoch types with checked monotonic transitions.
- Add typed `GraphRef`, `NodeRef`, and `EdgeRef` values/handles carrying database/graph identity and referent ID, with generation validation policy.
- Specify behavior for deleted referents, dropped graphs, copied reference values, compaction, and graph reopen/recovery.
- Remove public constructors/conversions that make accidental row→ID reinterpretation easy; raw ID construction remains controlled for codecs/tests only.
- Add identity audit lints/tests covering public signatures and internal conversion funnels.

### Non-goals

- No candidate-set API yet.
- No directionality.
- No cross-database globally routable identifiers.
- No UUID/string format promise for element IDs beyond the selected type contract.

### Acceptance evidence

- Public API inspection finds no `RowIndex`, raw row `u32`, or unchecked row-to-ID conversion.
- Compaction changes rows while all live stable IDs and reference values continue to resolve correctly.
- Deleted/dropped referents produce the selected invalid-reference GQLSTATUS on access but reference copying remains allowed.
- ID kind mixups are compile-time errors.
- Generation/epoch transitions have overflow and stale-state tests.
- Internal codecs use explicit audited constructors rather than broad public `new(raw)` paths.

### Tests and gates

- Compile-fail tests for ID kind and private row boundaries.
- Compaction/reference property tests.
- Delete/drop/recreate invalid-reference tests.
- Codec round-trip and malformed ID tests.
- Repository row-arithmetic script updates.
- Mutation tests around liveness/reference validation.

### Review focus

- No physical identity leaks.
- Reference invalidation semantics.
- Generation use is precise rather than over-invalidating.
- Codec/internal constructor containment.

### Stop conditions

- An existing public API cannot be removed without a compatibility promise; the EOL decision says remove it.
- Stable ID persistence semantics conflict with planned format 2.0.
- Reference values require unresolved type-system decisions; keep type descriptor opaque but identity-safe.

### Bridge and deletion

- Internal legacy `RowIndex` may remain as a private alias while call sites migrate.
- M04-PR02 deletes row-indexed public methods and completes the boundary.

<a id="m04-pr02"></a>
## M04-PR02 — Introduce Typed Generation-Bound Candidate Sets and Remove Row Bitmap APIs

- **Owner:** M04
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M04-PR01
- **Issues:** #1093
- **Commit scope:** `graph`

Close #1093 by replacing every public row-indexed bitmap producer with typed node/edge candidate sets whose storage representation is private and safe for set algebra.

### Scope

- Add sealed element-kind markers and `CandidateSet<Node>` / `CandidateSet<Edge>` (or equivalent distinct types).
- Bind each set to database/graph identity, immutable snapshot generation, element kind, and private bitmap/selection representation.
- Provide union/intersection/difference, cardinality, contains-by-ID, ID iteration, and internal row iteration only inside trusted graph modules.
- Replace `live_nodes`, `nodes_with_label`, `nodes_with_property_*`, edge counterparts, index-provider results, maintained candidate state, and optimizer adapters.
- Reject cross-graph, cross-generation, and cross-kind algebra with typed errors rather than silently translating.
- Delete public raw bitmap methods and repeated hand-written row→ID loops; update issue #1093 closure evidence.

### Non-goals

- No stable serialization format for candidate sets; they are snapshot-local ephemeral values.
- No public row iterator escape hatch.
- No query batch representation yet.
- No change to stable NodeId/EdgeId semantics.

### Acceptance evidence

- Repository public API contains no row-indexed `RoaringBitmap` return type.
- All consumers named in #1093 migrate to typed candidates or private projection APIs.
- Set algebra mismatch tests cover database, graph, generation, and kind.
- ID iteration remains correct before/after compaction and snapshot publication.
- Candidate-set performance is within reviewed bounds of raw bitmap operations for in-generation algebra.
- Issue #1093 can be closed with direct tests and docs rather than warning-only mitigation.

### Tests and gates

- Property tests comparing candidate algebra against ID `BTreeSet` reference.
- Compaction/generation mismatch tests.
- Compile-fail tests for node/edge kind mixing and private rows.
- Algorithms/query/index regression suite.
- Public API snapshot and row-arithmetic lint.
- Mutation tests for mismatch guards and ID resolution.

### Review focus

- No hidden raw-row escape.
- Generation and graph binding.
- All downstream consumers migrated.
- Performance evidence separates representation overhead from ID resolution.

### Stop conditions

- A downstream API fundamentally requires persistent candidate serialization; split a separately designed feature.
- Generation binding causes unacceptable query-plan invalidation; investigate resolver ownership, do not remove safety.
- PR exceeds size cap due unrelated algorithms cleanup; split adapters.

### Bridge and deletion

- No public compatibility bridge.
- A crate-private `RowCandidates` helper can exist but must not cross crate/public boundaries.

<a id="m04-pr03"></a>
## M04-PR03 — Add Explicit Directed and Undirected Edge Storage

- **Owner:** M04
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M04-PR01, M02-PR03
- **Issues:** None
- **Commit scope:** `graph`

Represent edge directionality as first-class state and update graph/type/mutation/index invariants for mixed graphs without encoding undirected edges as duplicate directed edges.

### Scope

- Define `EdgeDirectionality`/endpoint types with directed source+destination and undirected unordered endpoint pair, including self-loops.
- Extend edge store rows, adjacency indexes, creation/deletion/update changes, graph verification, compaction, and graph-type validation.
- For undirected edges, maintain one stable edge identity and adjacency visibility from both endpoints.
- Add directed/undirected/mixed graph-type constraints and endpoint-type validation.
- Define deterministic canonical storage ordering for undirected endpoints solely as representation, never semantic direction.
- Update in-memory snapshots/change events while explicitly deferring persisted format cutover to M09.

### Non-goals

- No parser/path pattern changes yet.
- No 1.x WAL/snapshot compatibility.
- No hyperedges or edges with more than two endpoints.
- No automatic conversion of existing directed edges to undirected.

### Acceptance evidence

- Create/read/delete/compact directed, undirected, mixed, parallel, and self-loop edges with one identity each.
- Incident/outgoing/incoming APIs have explicit tested semantics for each directionality.
- Graph type validation accepts/rejects endpoint ordering and types correctly for directed versus undirected definitions.
- No code path infers directionality from endpoint order or duplicated adjacency rows.
- Graph verification detects deliberate directionality/adjacency corruption in test fixtures.
- Change events can round-trip in memory and are ready for one unambiguous format 2.0 encoding.

### Tests and gates

- Model/property tests over random mixed multigraph mutations.
- Self-loop and parallel-edge regression tests.
- Adjacency and graph-type endpoint tests.
- Compaction and reference validity tests.
- Verification corruption fixtures.
- Mutation tests around directionality branches.

### Review focus

- Single identity for undirected edges.
- Adjacency semantics and self-loop deduplication.
- Graph-type endpoint correctness.
- No accidental persistence compatibility work.

### Stop conditions

- Current store layout cannot add directionality without an unsafe or ambiguous transitional state; use a clean internal version cut.
- Graph type semantics for undirected endpoint sets remain unresolved.
- Algorithms rely on source/target assumptions that require a separate adapter PR before merge.

### Bridge and deletion

- Existing directed-edge constructors may map explicitly to `Directed`; remove ambiguous two-endpoint constructors.
- Persisted encoding remains blocked until M09 and tests must not pretend 1.x files support undirected edges.

<a id="m04-pr04"></a>
## M04-PR04 — Implement Mixed-Edge Traversal, Pattern Orientation, and Predicates

- **Owner:** M04
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M04-PR02, M04-PR03
- **Issues:** None
- **Commit scope:** `gql`

Propagate directionality through graph read APIs, pattern matching, directed/source/destination predicates, algorithms projections, and candidate production with exact orientation semantics.

### Scope

- Add traversal primitives that request outgoing, incoming, either orientation, or incident semantics explicitly.
- Update edge-pattern matching for the seven non-empty combinations of left-directed, undirected, and right-directed orientations in full/abbreviated forms supported by the profile.
- Implement/verify directed, source, destination, and endpoint predicates over reference values.
- Update path construction to record traversal orientation separately from stored edge directionality.
- Update algorithms projections with explicit policy: directed, undirected, symmetrized, or reject unsupported mixed input.
- Update candidate producers and native procedures that currently accept string direction values.

### Non-goals

- No quantified/automata path rewrite; M07 owns it.
- No new graph algorithm semantics beyond projection policy.
- No parser redesign outside required edge patterns/predicates.
- No persistence.

### Acceptance evidence

- All edge pattern orientation combinations return exact expected matches on a compact mixed-graph fixture.
- Path values preserve node-edge alternation and occurrence orientation without duplicating undirected edges.
- Directed/source/destination predicates return correct three-valued/error behavior for invalid/null/non-edge inputs.
- Algorithms/procedures document and test mixed-graph projection policy.
- No remaining traversal code assumes every edge has source/destination fields.
- Conformance feature states for undirected support advance only with evidence.

### Tests and gates

- Exhaustive small mixed-graph edge-pattern matrix.
- Directed self-loop and undirected loop tests.
- Predicate type/null/invalid-reference tests.
- Algorithms projection policy tests.
- Existing path/query corpus differential tests.
- Parser fuzz if edge grammar changes.

### Review focus

- Orientation versus directionality distinction.
- Self-loop semantics.
- No silent mixed-graph reinterpretation.
- Feature claim/evidence precision.

### Stop conditions

- Any algorithm has ambiguous mathematical semantics on mixed graphs without a documented policy.
- Current path value cannot represent occurrence orientation; add a focused prerequisite type.
- Parser behavior diverges from verified GQL forms.

### Bridge and deletion

- Current row executor pattern code may remain until M07, but all edge access must use typed directionality APIs.
- Delete stringly direction parsing at graph/algorithm internals.

<a id="m04-pr05"></a>
## M04-PR05 — Finalize Directional Change Events and 2.0 Snapshot-Logical Model

- **Owner:** M04
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M04-PR03, M04-PR04, M03-PR04
- **Issues:** None
- **Commit scope:** `core`

Define version-independent logical changes and snapshot records for catalog objects, stable identities, and mixed edges so persistence 2.0 can encode one unambiguous model later.

### Scope

- Replace/extend logical `Change` variants with catalog ID, graph ID, stable element ID, explicit directionality/endpoints, typed property/schema/index/constraint references, and transaction metadata needed by 2.0.
- Define a logical snapshot model independent of rkyv/postcard/file-section layout.
- Ensure catalog and graph changes can be ordered and validated as one transaction bundle.
- Add invariants for referential ordering: parents/types/endpoints exist before dependent records and deletes occur in safe reverse order.
- Provide deterministic canonical encode-input ordering without selecting the final byte codec.
- Isolate legacy 1.x payload structs behind tests/archive-only code and mark them for deletion in M09-PR08.

### Non-goals

- No WAL/snapshot file writer or migration.
- No backward-compatible enum discriminants.
- No provider-specific derived accelerator payload as authoritative data.
- No compression/checksum choice.

### Acceptance evidence

- Random mixed-graph/catalog transactions round-trip through the logical model and rebuild equivalent in-memory state.
- Undirected edges have one unambiguous record and stable ID.
- Invalid referential order/missing parents/endpoints/type IDs are rejected by model validation.
- Catalog and graph generation transitions are explicit.
- No final file-format magic/version/codec assumption leaks into logical records.
- Legacy payload use is enumerated and assigned to M09-PR08 deletion.

### Tests and gates

- Property tests for transaction-bundle and snapshot-model round trips.
- Referential-order negative fixtures.
- Mixed edge/catalog/constraint/index model tests.
- Determinism/hash tests for canonical ordering.
- Current in-memory replay adapter tests.

### Review focus

- Semantic model versus byte format separation.
- Referential ordering and generation invariants.
- One catalog+graph transaction bundle.
- Legacy code containment.

### Stop conditions

- Logical records require final codec-specific representation.
- Catalog and graph changes cannot be replayed at one generation cut.
- A derived provider is currently commit-critical and cannot be made rebuildable without a separate decision.

### Bridge and deletion

- A temporary adapter may translate logical 2.0 changes to the current in-memory/1.x writer for tests only.
- It is not a compatibility promise and is deleted in M09.
