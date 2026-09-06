---
plan_id: F05-PR03
milestone: F05
initial_status: proposed
---

# F05-PR03 — Implement selective paths and materialize correct path values

**Milestone:** [F05: Finish paths, indexes and measured performance](Milestone-F05.md)  
**Dependencies:** [F05-PR02](Milestone-F05-PR-02.md)  
**Carries forward:** M07-PR04, M07-PR05; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-test-design`; `rust-performance`; `rust-memory-layout` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Implement selected ANY/shortest/count/group prefixes by endpoint partition and produce typed path values without dropping ties, predicates or identity.

## Start from what exists

The bounded executor and type descriptors are available. This PR combines selection and path-value materialization because an endpoint-only answer cannot validate the selected paths the user actually receives.

**Observed live entry points:** `crates/selene-gql/src/runtime`, `crates/selene-core/src`, `crates/selene-db/src/outcome.rs`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Partition candidate bindings by their stable start/end node pair. Compute path length according to the standard and select within each partition, not globally across a query.
2. Evaluate path-local search conditions before the final selective choice where the standard requires it. A rejected shortest topological route does not justify returning no match when a longer valid route exists.
3. Implement the selected prefix families, count=0 and tie/group behavior. For allowed nondeterministic choices, use a documented reproducible tie policy without falsely making it a portable ISO guarantee.
4. Materialize path values from stable alternating node/edge references and correct orientations. Shared predecessor/prefix representation is optional initially; add it only if measured retention/materialization cost justifies complexity.
5. Preserve cancellation and resource errors while collecting required ties/groups. Do not return the first handful of ties as a complete ALL SHORTEST answer.

## Acceptance and concrete regression cases

- [ ] Two endpoint partitions with shortest lengths 1 and 3 both contribute their own shortest results.
- [ ] A predicate rejects the length-1 path but accepts a length-2 path; selection returns the valid path instead of filtering an already-selected invalid shortest result.
- [ ] ALL SHORTEST preserves all qualifying shortest ties; counted shortest and counted shortest groups deliberately differ.
- [ ] Count zero, counts larger than the partition and unreachable endpoints follow the selected prefix semantics.
- [ ] Path values begin/end in nodes, maintain alternation and distinguish parallel edges; a zero-edge path still contains its endpoint node.
- [ ] Exhausted memory while accumulating ties fails the request rather than publishing a partial complete result.

## Validation and performance

Use exhaustive enumerate → qualify → partition → select on small bounded fixtures, plus independently authored unbounded-selective cases. Run the included selector reference tests as supplemental evidence, not as a parser or complete GQL conformance suite. Check declared path result types and deleted-reference access.

Measure discovery versus predecessor retention versus final materialization. Include many-tie, long-path and rejected-shortest workloads. Bound resource use without changing the requested semantics.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No graph-only distance cache that ignores automaton/history/predicates, global shortest filter, forced weighted interpretation or premature prefix-sharing rewrite.

## Bridge/deletion boundary

Old path value/result adapters are removed as the typed representation becomes authoritative. F05-PR04 removes legacy path dispatch.

## Standards and reviewer focus

§4.11.8; §4.15.2; §16.6; §§20.13–20.14; §22.4.

**Independent review question:** Is selection applied to the correct qualified path bindings in each endpoint partition, including all required ties?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
