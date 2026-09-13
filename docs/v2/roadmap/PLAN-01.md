---
plan_id: PLAN-01
initial_status: proposed
---

# PLAN-01 — Adopt the finish plan and remove mechanical planning blockers

**Outcome:** one current delivery program, aligned to the live code, with Luna executing coherent PRs and the orchestration model managing integration. No database behavior changes are needed here.

## Why this is a real prerequisite

The live plan validator binds counts, generated projections and candidate-part file inventories. Current delivery policy also specifies fixed file/net-line caps and an independent reviewer pair. The new scope cannot honestly be executed by silently ignoring those contracts. Adopt the replacement and update the affected tooling/policy together; afterward, routine caller discovery should not require another planning PR. [S01–S03, S11]

## Implementation sequence

1. Read current repository instructions, docs/v2 decisions, the live plan and recent merged work. Reconcile any changes since this review into the completed ledger and remaining scope. Do not rerun old M00/M01/M02/M03 work.
2. Place the supplied finish Markdown, examples, read-only review view and slim plan.json under `docs/v2/roadmap/`. Keep the existing plan.json location as the scheduling authority. Point `docs/v2/README.md` and operating/review entrypoints to the new start page. Preserve old plan/decisions as clearly historical material, not a second active queue.
3. Replace the old count/path-allowlist assumptions in `.github/scripts/check-v2-plan.py` and its tests with useful structural checks: unique IDs, valid dependency references/no cycles, resolvable local documents, issue ownership and valid completed-to-pending mapping. Remove the obsolete closed schema or reduce it to actual structural needs. Do not preserve fixed milestone/PR counts or a mandatory list of every production path.
4. Make D-021’s delivery change explicit: one coherent reviewable behavior; necessary callers/tests move with it. File/net-line totals are review information, not automatic stop conditions. Keep the repository’s existing per-source-file size/style/security gates unless this same PR deliberately and narrowly changes one.
5. Make D-022’s role change explicit: Luna edits/tests; the orchestrator owns integration and authorized Git/PR actions. One independent review is the default; add a focused second lens where a distinct durability/concurrency/semantic risk warrants it. Review/check evidence must still apply to the current change. Remove manual revision-ledger and exact-diff-inventory rituals without weakening ordinary branch protection or truthful test reporting.
6. Consolidate repeated local validation instructions into the shared guide. Retain the real Linux workspace PR gate and profile/API/safety checks. Do not automatically skip tests merely because a touched file is under docs: profile data, grammar fixtures, code generation and workflow scripts can change runtime behavior.
7. Validate the installed links/DAG, legacy mapping and issue owners, and run the revised plan validator’s positive and negative tests. The adoption PR must not falsely mark a product PR merged or auto-close #1093.

## Acceptance

- The completed 21 live items remain completed; candidate Parts 1/2 are retained as partial progress, not repeated tasks.
- Every one of the 65 live IDs maps to retained history or an explicit remaining completion owner. All seven current issues have a final owner.
- New PRs are not blocked just because a required caller was absent from a forecast path list.
- Local instructions, plan renderer/checker and PR role policy agree. No live entrypoint sends Luna back to an obsolete kickoff or names Luna only as a reviewer.
- JSON is a readable dependency/navigation index; PR Markdown is the behavioral contract. The HTML view is generated/read-only, not a parallel editable tracker.
- No product source, format compatibility or conformance claim is changed as a side effect of adopting the plan.

## Validation

Run the revised plan script and its tests, local link/JSON checks and the repository checks appropriate for scripts/workflows/docs. Include negative fixtures for a missing dependency, dependency cycle, duplicate ID, unresolved PR file and missing issue owner. Do not add another framework, schema service or exhaustive governance state machine.

Until this PR lands, existing repository gates still apply. It is not permission to disable a failing check during implementation.

## First handoff

Dispatch F01-PR01 next. Independent F02-PR01/02 and F03-PR01 owners may start once their source boundaries are agreed; do not parallelize multiple writers in candidate/storage ownership while its migration is unfinished.
