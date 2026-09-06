# Selene DB 2.0 review protocol

## Roles

**Implementer**

- Reads repository instructions, the program entry point, owning milestone,
  exact work item, current source, tests, and relevant issue evidence.
- Implements one bounded slice and runs its required gates.
- Returns the tested worktree and handoff without staging, committing, pushing,
  creating or updating a PR, reviewing, or merging.

**Orchestrator**

- Owns commits, pushes, non-draft PR creation and updates, CI polling, and the
  single consolidated comment that carries independent review results.
- Keeps the implementation and reviewer conclusions separate from its own.
- May merge only after every eligibility condition below is satisfied and the
  user has explicitly authorized merge.

**Independent read-only review**

- One independent read-only review is the default; add a focused second lens
  where a distinct durability, concurrency, or complex semantic risk warrants it.
- Reviewers inspect the exact diff, handoff, source context, tests,
  generated evidence, and CI at the same named head SHA.
- Each reviewer independently checks scope, architecture, semantics, diagnostics,
  performance evidence, failure behavior, and bridge deletion.
- Reviewers return PASS, FIX, or REPLAN without editing, approving, reacting,
  merging, or adopting the implementer or orchestrator's conclusions.

The repository owner resolves product decisions raised by REPLAN and performs
settings, protected-branch, archive, release, publication, and tag actions when
separately authorized.

## Verdicts

- **PASS:** the outcome, acceptance evidence, scope, and deletion obligations
  are satisfied at the reviewed head. PASS is evidence for merge eligibility;
  it is not a merge action or approval.
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

## Merge eligibility and required checks

The orchestrator may merge only when all of these conditions hold:

- the final reviewed head is unchanged;
- required checks report green for that exact head;
- the final review is Blocker/Major-clean;
- repository policy and branch protection permit merge;
- the PR scope and implementation worktree state are clean; and
- the user has explicitly authorized merge. Authorization may be a standing
  instruction for the active session.

A changed head voids PASS and requires review under the bounded loop. No role
may infer self-approval, auto-merge, release, publication, tagging, reactions,
or branch-protection mutation from PASS or merge authorization.

After M00-PR03 merges, the desired required contexts on `development` are:

- `fmt`
- `file-size cap (700 LOC)`
- `no-secret scan`
- `bench invocation lint`
- `rust compile and test`
- `2.0 plan contract`

Before M00-PR03, authenticated settings show only the first four contexts and
no required-review rule. Adding the two contexts is a post-merge owner/settings
action; this PR does not mutate branch protection. Independent read-only review
is the configured 2.0 review control, so the policy does not require a
GitHub self-approval that the acting account cannot provide.

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

Each cycle reviews one immutable head SHA. Reviewers examine that same head.
Cycle 1 may produce one batched repair for confirmed Blocker and
Major findings. Cycle 2 reviews the repaired head and must be
Blocker/Major-clean or return REPLAN. There is no third cycle. Minor and
Follow-up observations are tracked without changing the PR and do not trigger
the repair loop.

After an eligible authorized merge, update plan state from the merged
integration head. PASS on an earlier head is void if the PR changes.
