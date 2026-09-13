---
plan_id: F06-PR02
milestone: F06
initial_status: proposed
---

# F06-PR02 — Qualify release artifacts and complete the authorized 2.0 release

**Milestone:** [F06: Qualify and release 2.0](Milestone-F06.md)
**Dependencies:** [F06-PR01](Milestone-F06-PR-01.md)
**Carries forward:** M10-PR07; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-ci-release`; `rust-storage-durability`; `rust-test-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Qualify the actual distributable artifacts on supported native platforms and release only under the existing owner-authorized workflow.

## Start from what exists

The repository already separates routine Linux PR checks from heavier release/nightly work. Preserve that posture. This plan does not authorize tags, publishing, branch-protection changes or releases by itself. Source: S11.

**Observed live entry points:** `.github/workflows/release.yml`, `.github/workflows/nightly.yml`, `.github/workflows/ci.yml`, `Cargo.toml`, `Cargo.lock`

**Search hints, not verified current filenames:** `crates/selene-db/Cargo.toml`, `scripts/run-benches.sh`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Run the joined release candidate through native Linux/macOS lanes and other platforms only if they are actually part of the supported release matrix. No QEMU or cross-compilation locally or in GitHub; containers use the host’s native architecture.
2. Run the broad commit/checkpoint/recovery failure campaign, decoder fuzz/stress, concurrent reader/writer/retention cases and public API/selected-profile suite. Distinguish process-crash tests from filesystem/device power-failure guarantees.
3. Package each public crate in dependency order and smoke-test an external consumer against packaged contents. Inspect manifests, included files/licenses, feature resolution and examples without workspace path shortcuts.
4. Attach appropriate GitHub-consumable package/review artifacts and concise release notes with the agreed format/API/support boundary. The embedded Rust library does not need invented server binaries or Python wheels.
5. Present the evidence and unresolved issues to the orchestrator/owner. Execute publication/tag/release actions only with the separately applicable authorization; do not weaken a failed gate to meet a date.

## Acceptance and concrete regression cases

- [ ] All required native lanes run on real compatible hosts; unavailable lanes are stated as unavailable, not passed.
- [ ] Packaged crates build and their consumer smoke succeeds without unpublished path-only dependencies.
- [ ] No format-1 decoder/migration, old production executor or stale public bridge appears in release artifacts.
- [ ] Durable outcomes, required constraints and path/native semantics pass together on the candidate.
- [ ] Balanced performance/recovery/memory guards have reviewed results with no unexplained material regression.
- [ ] Release notes and generated claims match the actual tested support boundary and publication remains owner-authorized.

## Validation and performance

Use the existing release workflow and wrappers, with separate doctests and package checks. A dry-run publication validates packaging but is not publication and does not replace dependency-order/consumer validation. Record actual pass/fail/skipped results, not only a list of intended commands.

Run the selected release guard set and capacity/recovery cases, not an unbounded benchmark expedition. Isolate performance hosts from competing agent work and report remaining platform gaps.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No emulated/cross-built qualification, registry upload from a review PASS alone, fake binary product, unrequested binding release or broad last-minute dependency upgrade.

## Bridge/deletion boundary

No temporary bridges remain. Follow-up optimizations belong to a visible post-GA backlog with the release contract preserved.

## Standards and reviewer focus

§24 declaration boundary; deployment/artifact qualification is Selene product evidence.

**Independent review question:** Has the team tested the bytes adopters will consume, rather than only the repository workspace?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
