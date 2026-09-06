# Start here — resume the work, not the old kickoff

The current frontier is **M04-PR02 candidate cleanup**. Parts 1 and 2 are already implemented. The remaining graph-internal bridge deletion and downstream public-row deletion become [F01-PR01](Milestone-F01-PR-01.md) and [F01-PR02](Milestone-F01-PR-02.md).

Do not restart M00, recreate the profile, add another facade, or replace the transaction authority. The live code has these foundations; the uploaded archive’s early kickoff instructions are historical. See [current progress](01-CURRENT-PROGRESS.md).

## First execution

Land [PLAN-01](PLAN-01.md) to reconcile the roadmap and delivery policy. Then assign Luna F01-PR01, followed by F01-PR02. The latter owns final candidate-safety acceptance and #1093 closure.

After PLAN-01, independent owners can begin F02-PR01 (directory/store control), F02-PR02 (catalog declarations) and F03-PR01 (semantic analysis), provided shared catalog/facade changes have an integration owner. They do not wait for all of F01; the logical persistence codec later waits for the actual mixed-edge and value/reference contracts it consumes.

A single Luna executor should follow the highest-value ready item instead of simulating parallel work. Multiple Luna executors use separate worktrees and non-overlapping logical ownership; the orchestration model owns integration.

## Small context packet for Luna

Read repository instructions, the assigned PR, its stated dependencies and the relevant actual call path. Load its one-to-three focused skills from the skills checkout. Consult shared semantic or validation notes only for the boundary being changed. Do not load all historical milestones and all 77 skills into every implementation context.

Start by naming what already works, the first missing behavior and the regression that will prove the change. Then implement, test and return the tested worktree. Re-ground if the branch has advanced; merged behavior does not need to be redone merely because the package uses a different work-item ID.

## Where downstream projects can start

A facade-only in-memory smoke is useful now and is strengthened in F01-PR02. The first durable integration gate is F02-PR08; the native consumer gate adds vector/text/JSON and composite/key constraints. Neither gate pretends to be GA. These are useful handoff points for downstream adapter work, not new public compatibility promises or automatic release authorizations.

The [master roadmap](02-MASTER-ROADMAP.md) gives the exact dependency chain and [Luna guide](03-LUNA-EXECUTION.md) contains the dispatch prompt.
