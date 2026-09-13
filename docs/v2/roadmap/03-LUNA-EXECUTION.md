# Luna execution with an orchestration model

## Roles and authority

Luna is the implementation role chosen for this program. This package assumes no particular Luna API, context window, hidden capability or model family. The orchestration model selects ready work, supplies a bounded context packet, resolves cross-lane ownership and manages authorized Git/PR/integration actions. Repository policy still controls the actual runtime.

Luna reads code, implements and tests the assigned behavior, inspects its own diff and returns the worktree/evidence. Independent review is performed by another isolated agent or a human; self-inspection is not self-approval. One meaningful reviewer is the default after PLAN-01; a second lens is useful for distinct commit/recovery, concurrency or complex language-semantics risks, not a compulsory role checklist for every edit.

No review outcome grants publication, tagging, merge or permission-change authority. These remain subject to the applicable user authorization and repository controls.

## Context and skills

The skills repository contains 77 focused skills. Load the assigned PR’s one-to-three specialists, not the entire collection. Read `skills/<name>/SKILL.md` from the available checkout or installed path; `$skill-name` invocation is only a convenience if the actual harness supports it. The skills repo does not configure agent roles or grant new tools/permissions. [S10]

Use repository-grounding/source-verification to resolve actual code or version-sensitive facts when needed. Use rust-nextest for runner mechanics, rust-test-design for evidence design and rust-review for semantic review. Durability work uses rust-storage-durability; numerical vector work uses rust-numerics-simd. Do not substitute guessed names such as rust-testing or rust-concurrency.

Keep one shared context note for the current lane: established contracts, changed interfaces, first remaining uncertainty and next executable step. Refresh it when ownership/session changes, not after every small edit. A memory service is optional; this package requires none.

## Dispatch prompt

```text
Implement <PR-ID> from the current Selene DB 2.0 finish plan.

Read repository instructions, the assigned PR file, its relevant dependencies,
and the actual source/callers before editing. Use its named skills. Keep the
completed foundations; do not reimplement work merely because an old plan
uses different IDs or paths.

Deliver the PR's observable outcome and concrete regression cases. Choose
internal interfaces from current code; navigation paths and samples are not
closed implementation specifications. Include necessary adjacent callers and
tests. Keep unrelated cleanup and feature additions out.

Preserve stable semantic IDs, private graph rows, serializable single-writer
publication, one authoritative WAL, format-2-only persistence, accurate GQL
semantics and the repository unsafe-code prohibition.

Start with the smallest meaningful regression. Run focused tests and required
repository gates. Report actual results and unavailable/unrun checks; never
replace failures by relabeling them or regenerating expected results.

Return the tested worktree, a brief implementation summary, checks and remaining
risks. The orchestrator owns authorized Git/PR/integration actions. Ask Justin
only about a material product, compatibility, durability, platform or authority
choice that source inspection cannot resolve.
```

Supply the actual PR file and relevant code, not only this generic prompt. The PR’s cases and bridge deletion are the substantive task.

## Implement through a small evidence loop

Start by stating the current behavior, the missing outcome and one fixture that separates correct from incorrect behavior. Inspect the mutation/query call path before choosing a type or module. Implement that slice and run its focused test; then expand to the named boundary cases and affected consumers.

Use failure feedback diagnostically. A compile failure from a moved caller is normal migration work. A failing semantic reference means investigate the implementation and the reference independently. A larger-than-forecast diff means inspect cohesion; it is not automatically a product decision. Do not mask flakes with retries, remove a test because it exercises a hard boundary or widen unsupported semantics to get green output.

The suggested code samples teach narrow boundaries. They are not production patches or compulsory interfaces. A better implementation is welcome when it preserves the outcome and improves caller simplicity/evidence.

## Decisions Luna can make

Routine reversible choices include module extraction, private helper signatures, collection selection backed by measurements, fixture layout and which affected callers move together. Keep the diff reviewable and explain a consequential tradeoff in the handoff; no separate design document is required.

Escalate an incompatible public/stored-value contract, a changed durability or isolation guarantee, unsupported native filesystem guarantees, a requested feature that conflicts with the retained release scope, a new authority/dependency inversion, an unsafe-code exception or an action requiring missing owner permission. Before asking, resolve discoverable facts and present the concrete alternatives and consequences. Do not ask Justin which current file contains a symbol.

## Parallel execution

Each PR has one implementation owner. Separate worktrees do not make edits to the same transaction/value/registry invariant independent. The orchestrator assigns a shared-interface owner, integrates producers before their consumers and reruns combined tests when lanes meet. A draft interface is useful for research, but does not satisfy a merged dependency.

Builders and benchmark processes contend for CPU and memory. Serialize performance comparisons and budget build/test concurrency separately. Unavailable subagents are not a blocker: the same work can run serially with one Luna executor and independent review later.

## Compact handoff

```text
PR / outcome:
What changed and why:
Public or durable contract changes:
Checks actually run and results:
Unrun checks / known failures:
Temporary bridge still present and deletion owner:
Material question, or next ready step:
```

The reviewer returns concrete blocking findings or acceptance of the current change; ordinary corrections remain in the same PR. Reopen review for changed risk-bearing code, not through a manually maintained revision packet. Avoid a new round of broad research after a local fix unless it exposes a real design conflict.
