---
plan_id: F04-PR07
milestone: F04
initial_status: proposed
---

# F04-PR07 — Restore vector retrieval through the stable native boundary

**Milestone:** [F04: Deliver batch execution and native retrieval](Milestone-F04.md)  
**Dependencies:** [F04-PR06](Milestone-F04-PR-06.md)  
**Carries forward:** M10-PR02; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-numerics-simd`; `rust-storage-durability`; `rust-test-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Expose existing vector values, exact search and supported ANN modes through catalog-owned registrations and typed batch results, with honest exact/approximate contracts.

## Start from what exists

Lower vector search already exists and participated in candidate migration. Reintegration should reuse those kernels and preserve missing/non-vector skip behavior; it is not a mandate to add a new ANN package, GPU backend or embedding service.

**Observed live entry points:** `crates/selene-graph/src/vector_search.rs`, `crates/selene-graph/src/vector_search`, `crates/selene-gql/src/runtime/builtins/vector_search_ann.rs`, `crates/selene-db/src/lib.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Bind vector index declarations to graph/property/metric/dimension identity and rebuildable lifecycle. Validate candidates using the pinned graph; leave generic candidate binding liveness-only.
2. Adapt exact and ANN procedure signatures/results to F04-PR06. Document filter semantics, k behavior, tie order and metric direction; an approximate path must be explicitly selected or already part of the supported API contract.
3. Keep an exact reference path for correctness and ANN recall measurement. Filter-before/after-search behavior must be specified, especially where a selective candidate set would otherwise return fewer than k live results.
4. Cover insert/update/delete, rollback, graph replacement, recovery and rebuild invalidation. Derived vector state never becomes an authoritative source of graph values.
5. Add a small agent-memory-style retrieval fixture with deterministic vectors and graph-scoped metadata, no model downloads. Exercise typed results and restart after the durable lane is available.

## Acceptance and concrete regression cases

- [ ] Dimension mismatch, empty vectors, non-finite values and unsupported metrics follow the existing documented contract; no silent truncation or padding.
- [ ] Absent/non-vector properties are skipped by scoring while live-ID binding still succeeds.
- [ ] Exact top-k agrees with an independently computed metric fixture, including ties and k=0/greater than available candidates.
- [ ] Stale/deleted candidates never reappear from an ANN index after mutation or reopen.
- [ ] Filtered ANN results obey the declared approximation/filter policy; exact APIs never silently fall back to ANN.
- [ ] Rebuild failure cannot produce apparently complete exact results from stale derived state.

## Validation and performance

Run numerical kernel tests, exact/ANN reference fixtures, candidate lifecycle tests and facade/native integration. Record which metric computations share a kernel with the reference and add at least one independently derived numeric example.

Measure exact latency and ANN recall/latency together over representative dimensions/selectivity. Include memory/build/rebuild cost and CPU fallback. SIMD optimization stays within the workspace unsafe-code policy.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No new GPU release blocker, embedded model service, unvalidated approximate uniqueness/filter proof or change to VectorCandidateSet meaning.

## Bridge/deletion boundary

Old vector facade/procedure adapters are deleted after migration. Runtime accelerator caches remain derived and generation-validated.

## Standards and reviewer focus

Selene native extension profile; §4.4 value semantics and §15 typed procedure boundary still apply.

**Independent review question:** Can a caller tell when results are approximate, filtered, incomplete because of a declared limit, or invalid due to stale state?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
