# Selene DB 2.0 operating guide

## Context order

1. Read `AGENTS.md` and prove repository, branch, base, tree, prerequisites,
   issue state, and applicable instructions.
2. Read the [program entry point](README.md) and
   [finalized decisions](decisions/finalized.md).
3. Read the owning [milestone](roadmap/milestones.md) and exact work item.
4. Inspect current source, tests, benchmarks, issue evidence, and prior merged
   handoffs at named revisions.

Do not load broad research history by default. Follow the work item's anchors
when a source fact or design question requires it.

## Source inspection

Before editing, find every current caller and relevant failure test at the
owned seam. Include public/internal APIs, procedures, algorithms, docs,
examples, fuzz targets, persistence, generated evidence, and benchmarks as the
slice requires. Name temporary bridges and deletion owners. A source fact that
breaks the boundary returns REPLAN.

## Anti-drift rules

- No opportunistic refactor, compatibility alias, dual format, old public
  facade, or hidden fallback executor.
- No crate outside the finalized decisions and no dependency upgrade without
  direct technical need and evidence.
- No extension syntax presented as an ISO feature.
- No benchmark claim from a quick, noisy, or unmatched fixture.
- No manual edit to deterministic projections or future generated profile
  evidence.
- No warning suppression, weaker gate, or changed status code to make a test
  pass.
- No tracked instruction may depend on untracked working material.

Independent evidence work may run in parallel. One writer owns integration;
agents do not edit the same seam concurrently.

## Worktree handoff and review

Use the work item's conventional scope and keep one invariant within D-021.
The implementer edits and tests, then returns the worktree and handoff. It does
not stage, commit, push, create or update a PR, submit a review, or merge. The
orchestrator owns those Git and GitHub mutations, including the non-draft PR and
one consolidated comment containing the independent reviewers' conclusions.

Two read-only reviewers independently inspect the same immutable head. The
orchestrator may merge only after that head remains unchanged, required
exact-head checks are green, final review is Blocker/Major-clean, repository
policy permits the merge, scope and worktree state are clean, and the user has
explicitly authorized it. A changed head voids PASS. Self-approval, auto-merge,
release, publication, tagging, reactions, and branch-protection changes remain
separate and are not authorized by PASS.

M00-PR03 installs the CI compile lane, validator negative fixtures, PR template,
branch-protection instructions, and deterministic `cargo-about` pin.

The [required handoff](review-protocol.md) lists exact commands, results,
skips, evidence paths, deviations, bridge status, risks, and questions. “All
tests pass” is not a reproducible handoff.
