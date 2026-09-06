---
plan_id: F01-PR01
milestone: F01
initial_status: proposed
---

# F01-PR01 — Finish the graph-internal candidate migration

**Milestone:** [F01: Finish graph identity and mixed topology](Milestone-F01.md)  
**Dependencies:** [PLAN-01](PLAN-01.md)  
**Carries forward:** M04-PR02; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-api-design`; `rust-test-design`; `rust-performance` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Delete the remaining graph-internal `CandidateSet::{Node,Edge}::trusted_rows` bridge while retaining the already-landed identity and producer work.

## Start from what exists

Live M04-PR02 Parts 1 and 2 are implemented. Candidates contain stable-ID/typed-row pairs and retain physical-layout and mutable-workspace tokens. Both trusted_rows methods still exist. This is the remaining Part 3A outcome, not a candidate-system rewrite. Source: S03/S04.

**Observed live entry points:** `crates/selene-graph/src/candidate_set.rs`, `crates/selene-graph/src/graph.rs`, `crates/selene-graph/src/store.rs`, `crates/selene-graph/src/text_index.rs`, `crates/selene-graph/src/json_search_candidates.rs`, `crates/selene-graph/src/vector_search/score_candidate_batch.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Trace both bridge definitions and every current consumer, including text, JSON, exact vector scoring and approximate vector filtering. Use the old 13-file list as navigation, not a closed edit allowance after PLAN-01.
2. Choose graph-owned operations that accept typed candidates and return domain results or stable IDs. A private validated view may be used only inside the graph implementation if it cannot expose raw rows or outlive its validating snapshot; do not rename the old bridge and call that deletion.
3. Preserve compile-time node/edge separation, lower GraphId, graph generation, both layout-token checks and live forward/reverse ID pairing. Never bind candidates by copying a generation number from another graph.
4. Keep generic stable-ID binding liveness-only. Preserve the independent vector rule that skips absent/non-vector properties. Leave the unbound stable-ID VectorCandidateSet helper intact.
5. Delete the bridge methods and their stale comments, then exercise all migrated search/read paths. Keep the downstream public raw-row APIs only until F01-PR02; do not close #1093 here.

## Acceptance and concrete regression cases

- [ ] A candidate from an independent same-ID, same-generation graph is rejected rather than interpreted as local rows.
- [ ] Compaction/remap invalidates the old layout binding; a fresh stable-ID bind yields the same surviving IDs. A retained old immutable snapshot can still use its own candidates.
- [ ] Delete, reset, recovery, replacement and mutable-workspace changes preserve or remint tokens according to the existing lifecycle tests.
- [ ] Duplicate IDs, tombstones and absent IDs produce the existing canonical live-ID set; missing vectors do not make generic binding fail.
- [ ] Text, JSON and vector result IDs, score semantics and ordering match their pre-change contracts; a source/API check finds neither trusted_rows method nor an equivalent raw-row escape.

## Validation and performance

Run candidate lifecycle/algebra and migrated graph search tests, then affected graph/GQL consumer tests. Retain compile-fail kind-separation checks and API documentation checks. Add a regression that would fail if validation were accidentally replaced with graph-ID-only checking.

Measure candidate construction, repeated validation and complete search operations separately. The current validate_for scans entries; that is a cost to quantify, not permission to remove checks. Compare stable-ID resolution costs with the previous path on sparse and dense candidate sets.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No new serialization, public row view, generation bypass, lower-to-facade dependency, unrelated map replacement or removal of VectorCandidateSet.

## Bridge/deletion boundary

Only existing downstream raw-row surfaces may remain, with F01-PR02 as deletion owner. This PR does not complete old M04-PR02.

## Standards and reviewer focus

§4.3.1 object identity; §4.4.4 reference validity. Candidate layout safety is an implementation invariant, not a separate ISO feature.

**Independent review question:** Can any newly introduced helper expose unchecked rows, or can a valid old snapshot be incorrectly rejected simply because a newer snapshot exists?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
