---
plan_id: F05-PR07
milestone: F05
initial_status: proposed
---

# F05-PR07 — Resolve measured read-path regressions with balanced evidence

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)  
**Dependencies:** [F01-PR02](Milestone-F01-PR-02.md)  
**Carries forward:** M08-PR06; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** #1137  
**Focused skills:** `rust-performance`; `rust-memory-layout`; `diagnosis-loop` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Make a measured read/write/memory tradeoff for hot lookup maps and candidate validation, and install a small balanced regression set. Close #1137 with evidence, not an automatic map revert.

## Start from what exists

Issue #1137 reports read regressions of roughly 8–81% across a prior map migration while also reporting substantial write gains. Those are issue-reported results, not measurements rerun in this review. The unexplained secondary effects in unaffected maps make a blanket causal claim unsafe. Source: S07.

**Observed live entry points:** `crates/selene-graph/src/store.rs`, `crates/selene-graph/src/graph.rs`, `crates/selene-graph/src/candidate_set.rs`, `crates/selene-testing/src/bench_fixtures.rs`

**Search hints, not verified current filenames:** `scripts/run-benches.sh`, `BENCHMARKS.md`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Reproduce the current symptom on a controlled native host using the repository benchmark wrapper and unchanged fixtures. Establish current costs first; old issue measurements guide investigation but are not the current baseline.
2. Profile string-key label maps, stable-ID lookup maps, candidate validation and complete facade query/mutation paths. Distinguish tree lookup, copy-on-write update, cache/layout effects and repeated validation.
3. Try the smallest targeted representation change supported by the profile: different maps for read-hot lookup versus structurally shared mutation state may be appropriate. Keep candidate provenance/liveness validation intact.
4. Compare reads, writes, mixed workloads, clone/snapshot cost and memory on the same inputs/toolchain/hardware. Reject a local nanosecond win that creates an unacceptable end-to-end or memory regression.
5. Record the selected tradeoff and retain a compact read/write guard set. A defensible no-code-change conclusion can close the investigation only with explicit acceptance of the measured tradeoff, not with a fabricated performance win.

## Acceptance and concrete regression cases

- [ ] All candidate identity/liveness and mutation/recovery correctness fixtures remain unchanged and passing.
- [ ] Read-hot node fetch, label lookup and typed index point lookup appear in the guard set alongside create/update/delete/clone or mixed mutation.
- [ ] Benchmarks exercise actual consumption rather than optimizing away a returned lookup value.
- [ ] Sparse/deleted IDs and many labels expose representation assumptions not visible on tiny contiguous fixtures.
- [ ] Repeated candidate validation costs are measured without replacing checked operations with an unchecked raw-row iterator.
- [ ] Any accepted tradeoff is stated with absolute times, relative changes, variability, memory and untested hardware limitations.

## Validation and performance

Run correctness tests before comparative benchmarks. Use scripts/run-benches.sh and inspect its supported arguments; avoid cargo bench --workspace. Benchmark runs are serialized on the host so another agent’s build does not contaminate the comparison.

This PR is the measurement task. No fixed speedup target is invented. Recheck the chosen guard set at RC because later batch, path and index changes can alter the tradeoff.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No blanket dependency rollback, speculative hash-map replacement, unsafe optimization, weakening layout validation, write-only acceptance or comparison to an unrelated stale baseline.

## Bridge/deletion boundary

No compatibility bridge should result. Remove experimental implementations not selected by evidence; do not retain a configuration matrix of unproven alternatives.

## Standards and reviewer focus

Performance is an implementation property; selected language/value/visibility invariants remain fixed.

**Independent review question:** Is the claimed benefit supported by a controlled comparison of the whole relevant workload rather than one favorable microbenchmark?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
