# Work-item contract

The [machine plan](plan.json) is the complete structured contract. The two
[Markdown](work-items-00-04.md) [registers](work-items-05-10.md) are generated
projections for review. A future plan edit must update the machine record and
regenerate the projections with:

```bash
python3 -B .github/scripts/check-v2-plan.py --root . --write-projections
```

## Required record

Every work item must remain standalone and carry:

- ID, owner milestone, number, title, outcome, risk, size, and commit scope;
- exact work-item dependencies and owned issues;
- clause, feature, implementation-defined, and design anchors as identifiers
  or paraphrases only;
- primary crates and expected touchpoints;
- in-scope work, non-goals, and target design;
- acceptance evidence, focused tests, risk gates, and performance evidence;
- documentation or generated evidence obligations;
- reviewer focus, stop conditions, and bridge/deletion ownership;
- a file and explicit anchor in the tracked Markdown projection.

Paths are starting points, not permission to edit every listed file. Current
source and tests decide the narrowest correct change.

## Entry and execution

Before editing, prove the exact repository, base SHA, branch, clean or preserved
worktree state, merged prerequisites, current issue ownership, source symbols,
and commands. A false prerequisite or source contradiction returns REPLAN.

Implement one invariant. Add behavior evidence before broad migration, preserve
failure and lifecycle semantics, delete the bridge assigned to the slice, and
run focused gates before package and full gates. An unrun gate is reported as
unrun with the reason.

## Handoff and merge eligibility

The implementer returns a tested worktree without staging, committing, pushing,
creating or updating a PR, reviewing, or merging. Its handoff uses the fields in
the [review protocol](../review-protocol.md), names every skipped gate and
deviation, and assigns every remaining bridge. The orchestrator owns Git
history, the non-draft PR, consolidated independent-review comments, and any
eligible authorized merge.

Two read-only reviewers return PASS, FIX, or REPLAN for one exact head. Merge
eligibility requires an unchanged final reviewed head, green required exact-head
checks, Blocker/Major-clean final review, repository-policy permission, clean
scope and worktree state, and explicit user authorization. A changed head voids
PASS.

Stop for REPLAN when a locked decision, dependency, safety assumption,
conformance boundary, issue owner, or PR-sized scope is invalid. Do not absorb a
second concern to avoid the stop.
