# Selene DB 2.0 review protocol

## Roles

**Implementation agent**

- Reads repository instructions, the program entry point, owning milestone,
  exact work item, current source, tests, and relevant issue evidence.
- Implements one bounded slice and runs its required gates.
- Opens a non-draft PR to `development`, writes the handoff, and stops.
- Never merges, self-approves, reacts, changes protection, tags, or publishes.

**Assistant reviewer**

- Reviews the exact diff, handoff, source context, tests, generated evidence,
  and CI at named SHAs.
- Checks scope, architecture, semantics, diagnostics, performance evidence,
  failure behavior, and bridge deletion.
- Returns one verdict: PASS, FIX, or REPLAN.

**Repository owner**

- Resolves product decisions raised by REPLAN.
- Alone merges after PASS and green required checks.
- Performs protected branch, archive, tag, release, and publication actions.

## Verdicts

- **PASS:** the outcome, acceptance evidence, scope, and deletion obligations
  are satisfied. PASS authorizes an owner decision; it does not merge.
- **FIX:** the plan remains valid and one bounded repair batch can correct the
  same PR. Findings name severity and required evidence.
- **REPLAN:** implementation facts invalidate the boundary, dependency order,
  locked decision, or safety/conformance assumption. Coding stops. The handoff
  records evidence, affected IDs, alternatives, the proposed boundary change,
  and work safe to retain.

## Review order

1. Base, dependency, issue owner, scope, and unrelated-change check.
2. Facade/catalog, identity, context, compiler, execution, and durability
   invariants relevant to the slice.
3. Types, nulls, duplicates, order, statuses, side effects, cancellation,
   atomicity, and recovery behavior.
4. Focused, model/differential, failure, fuzz, crash, benchmark, and generated
   evidence required by risk.
5. Temporary bridge ownership and deletion assigned to this work item.
6. Public API, persisted format, extension, and conformance-claim boundaries.
7. Diagnostics, resources, docs, CI, package, and release effects.

## Required handoff

```text
Plan ID:
PR URL:
Base SHA / Head SHA / commits:
Outcome delivered:
Files/subsystems changed:
Public API and persisted/profile changes:
Commands and results:
Benchmarks/fuzz/crash evidence:
Decisions and deviations:
Temporary bridges and deletion owner:
Known risks/follow-ups:
Reviewer questions:
```

Missing commands are not green. The handoff states why each was skipped.

## Finding severity

- **Blocker:** data loss, atomicity/recovery error, claim overstatement,
  unsafe public identity, security/release bypass, or wrong work item.
- **Major:** incorrect semantics/status/type/order, architectural leakage,
  missing required evidence, or material unexplained regression.
- **Minor:** non-blocking maintainability, documentation, or test clarity,
  recorded without changing the PR.
- **Follow-up:** non-blocking work outside scope, assigned to a named item or
  issue.

## Bounded review loop

Each cycle reviews one immutable head SHA. Cycle 1 may produce one batched
repair for confirmed Blocker and Major findings. Cycle 2 reviews the repaired
head and must be Blocker/Major-clean or return REPLAN. There is no third cycle.
Minor and Follow-up observations are tracked without changing the PR and do not
trigger the repair loop.

After owner merge, update plan state from the merged integration head. PASS on
an earlier head is void if the PR changes.
