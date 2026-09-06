---
plan_id: F04-PR08
milestone: F04
initial_status: proposed
---

# F04-PR08 — Restore text, JSON and maintained providers through the same boundary

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)  
**Dependencies:** [F04-PR06](Milestone-F04-PR-06.md)  
**Carries forward:** M10-PR03; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-storage-durability`; `rust-test-design`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Expose existing BM25/text and JSON retrieval plus maintained-provider lifecycle through catalog declarations, typed candidates and batch outcomes.

## Start from what exists

Text/JSON helpers and provider recovery already exist below the facade. Candidate Part 2 deliberately reserves runtime attachment before recover callbacks. Preserve that safety property during reintegration; do not revert to callback-first attachment.

**Observed live entry points:** `crates/selene-graph/src/text_index.rs`, `crates/selene-graph/src/text_search.rs`, `crates/selene-graph/src/json_search.rs`, `crates/selene-gql/src/runtime/builtins/text_search.rs`

**Search hints, not verified current filenames:** `crates/selene-graph/src/candidate_state.rs`, `crates/selene-graph/src/recover.rs`. Locate the owning symbols before editing.

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Connect declarations and runtime provider instances through F04-PR06 and the existing lifecycle services. Keep primary graph values authoritative and accelerator state rebuildable.
2. Preserve tokenizer/analyzer/version and BM25 corpus-statistics identity. A filtered query must use the documented scoring population, not accidentally recompute corpus statistics on each candidate subset.
3. Adapt JSON search and typed candidates without inventing expression-index support before F05-PR06. A correct scan is acceptable until that optimization lands; a silently incomplete index answer is not.
4. Handle create/update/delete, rollback, graph reset/replacement, reopen and provider rebuild with generation-safe attachment and cleanup on callback failure/cancellation.
5. Add deterministic memory-document/text/JSON examples through the facade, including graph-scoped filters, result schemas and diagnostic behavior.

## Acceptance and concrete regression cases

- [ ] Text updates/removals, deletes and rollback produce the same answer as rebuilding from authoritative graph values.
- [ ] Tokenizer/analyzer changes invalidate stale derived indexes rather than mixing incompatible statistics.
- [ ] Filtered text ranking follows its declared corpus-statistics policy and stable tie handling.
- [ ] JSON missing versus null, scalar type distinctions and malformed path inputs preserve query semantics.
- [ ] A recover callback cannot re-enter a partially attached runtime; failure/cancellation leaves no half-registered provider.
- [ ] Optional provider unavailability is visible, while a provider required for active constraints cannot be silently skipped.

## Validation and performance

Run text/JSON exact-scan comparisons, provider lifecycle and callback-reentry tests, then native/facade scenarios. Add durable restart integration once F02 is ready; missing integration evidence remains explicit until then.

Measure text/JSON query cost, build/rebuild time, filter selectivity and retained memory. Avoid claiming an index acceleration when a new path still scans the entire graph.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No GIN containment index, authoritative accelerator snapshots, callback under an excluding lock, silent best-effort required provider or external search service.

## Bridge/deletion boundary

Old builtin/provider adapters and duplicate registration paths are removed. JSON expression-index planning remains F05-PR06, not an untracked follow-up.

## Standards and reviewer focus

Selene native extension profile; §4.4 values; §15 procedure effects; §4.6 visibility.

**Independent review question:** Are derived search structures exact with respect to their declared contract across every mutation and recovery path?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
