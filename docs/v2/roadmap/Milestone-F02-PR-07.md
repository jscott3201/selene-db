---
plan_id: F02-PR07
milestone: F02
initial_status: proposed
---

# F02-PR07 — Close recovery classification and failure evidence

**Milestone:** [F02: Bring durable embedding forward](Milestone-F02.md)  
**Dependencies:** [F02-PR06](Milestone-F02-PR-06.md)  
**Carries forward:** M09-PR07; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-storage-durability`; `rust-test-design`; `rust-review` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Finish one non-destructive recovery state machine with explicit corruption, unsupported-format and resource-error outcomes, and verify it across commit/checkpoint failures.

## Start from what exists

The earlier durable slice supplies an actual open pipeline. Extend it rather than writing a separate recovery engine for tests. Existing lower tail-repair behavior must be reviewed against the new authority protocol; the fact that a record is last does not prove it was unacknowledged. Source: S08.

**Observed live entry points:** `crates/selene-persist/src/recovery.rs`, `crates/selene-persist/src/recovery`, `crates/selene-persist/src/wal_tail.rs`, `crates/selene-persist/src/reader.rs`, `crates/selene-db/src/database.rs`, `crates/selene-testing/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Make recovery phases explicit: anchor and lock, inspect format/control, validate lineage, load selected snapshot, replay complete transactions, rebuild necessary derived state, check invariants, then publish the facade.
2. Classify foreign store/epoch, stale or missing manifest, sequence gaps/overlaps, incomplete permitted unsealed tail, integrity failure, structural corruption, unsupported semantic profile and bounded-resource exhaustion.
3. Keep recovery non-destructive by default. Any authorized torn-tail repair must have protocol evidence that the discarded bytes cannot contain an acknowledged transaction; ambiguous damage fails closed and remains available for inspection.
4. Validate catalog ownership, graph types, stable ID high-water values, edge endpoints, required constraint indexes and procedure/accelerator declarations before writes. Optional derived data may rebuild or degrade only through an explicit policy and visible status.
5. Use the real phase hooks to generate fault cases. Expose a read-only verification/report path that shares invariant checks without opening the store for mutation.

## Acceptance and concrete regression cases

- [ ] Every previously acknowledged transaction is present after supported recovery; definite cancellations are absent; indeterminate operations match one allowed whole-transaction outcome.
- [ ] Interior/sealed corruption and bad authoritative checksums are not relabeled as a harmless torn tail.
- [ ] Gaps, overlaps, cross-store artifacts and missing required files fail before publishing any database handle.
- [ ] A decoder limit failure terminates predictably without OOM, infinite loop or partial mutation.
- [ ] Rebuild failure for a required constraint blocks writes; optional ANN/text rebuild failure never silently changes an exact query into incomplete results.
- [ ] Verify/open do not alter damaged artifacts in the default mode, and diagnostics identify phase and artifact without exposing unrelated filesystem information.

## Validation and performance

Run a compact deterministic failure matrix in PR validation. Run broad process-crash, corruption mutation, fuzz and native-platform campaigns for the milestone/RC. Label process termination as process-crash evidence, not a simulated loss of OS page cache or proof of device power-loss behavior.

Measure recovery at small/large WAL suffixes, many graphs and required-index rebuilds. Keep replay linear where expected and account for peak memory and rejected-input work.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No salvage command, best-effort authoritative replay, acceptance of unknown authoritative records, silent profile downgrade or 1.x import.

## Bridge/deletion boundary

One authoritative recovery pipeline remains. F02-PR08 deletes old entry points after cross-layer acceptance.

## Standards and reviewer focus

§4.6; §8.3–8.4; §23. Recovery mechanisms are not prescribed by ISO, but observable atomicity and diagnostics still apply.

**Independent review question:** Could any corrupt state be made to look healthy by dropping a record, graph, index or warning?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
