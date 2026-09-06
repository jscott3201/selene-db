---
plan_id: F02-PR02
milestone: F02
initial_status: proposed
---

# F02-PR02 — Unify catalog metadata for constraints, indexes and native registrations

**Milestone:** [F02: Bring durable embedding forward](Milestone-F02.md)  
**Dependencies:** [PLAN-01](PLAN-01.md)  
**Carries forward:** M08-PR01; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).  
**Issue closure:** None; do not close another PR’s issue.  
**Focused skills:** `rust-api-design`; `rust-storage-durability`; `rust-test-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Make durable descriptor ownership explicit now, so persistence and native reintegration do not invent incompatible private registries later.

## Start from what exists

The catalog/facade are already implemented. This PR extends that ownership to the remaining index and constraint descriptors and establishes the registration data contract consumed by F04-PR06. It does not reimplement schema or named-graph lifecycle.

**Observed live entry points:** `crates/selene-db/src/catalog.rs`, `crates/selene-db/src/catalog_snapshot.rs`, `crates/selene-db/src/catalog_stage.rs`, `crates/selene-db/src/transaction.rs`, `crates/selene-catalog/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Inspect existing lower index, graph-type, provider and native procedure registries. Separate durable declarations from runtime handles, code pointers and derived accelerator state.
2. Add stable catalog identities, graph/type ownership, generation/dependency information and an explicit lifecycle for declarations: building/inactive versus validated/usable. These states must not make an unenforced constraint look active.
3. Define index targets as analyzed target descriptions with semantics/profile identity, not opaque closures. Reserve the expression representation that F05-PR06 completes; unsupported targets remain rejected.
4. Use the existing detached catalog draft and outer publication for descriptor create/drop/replace. Capture dependency invalidation so dropping an owned object cannot leave an advertised usable index or procedure binding.
5. Expose introspection and logical persistence records. Existing enforcement remains active until its replacement is complete; do not introduce a second authoritative registry.

## Acceptance and concrete regression cases

- [ ] Create/drop/rollback of an index or constraint declaration changes catalog and ownership together.
- [ ] Duplicate names, wrong graph/type owners and dangling dependencies fail with a stable structured diagnostic.
- [ ] A building or failed index is never selected by the planner or used as a complete constraint proof.
- [ ] Catalog generations invalidate dependent plans while an unrelated data publication does not masquerade as schema replacement.
- [ ] Logical descriptor serialization inputs contain stable IDs and declarative configuration, not pointer addresses or physical rows.
- [ ] An existing uniqueness rule stays enforced during migration; declarations without a supported implementation cannot be activated.

## Validation and performance

Run catalog lifecycle/draft rollback, dependency invalidation and profile tests. Add introspection fixtures and a lower-registry-to-catalog consistency test. Catalog-only tests are not sufficient evidence for constraint activation; F05-PR05 supplies that behavior.

Measure descriptor lookup and catalog snapshot clone/publication overhead at realistic graph/index counts. Avoid copying accelerator payloads into catalog snapshots.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No independent publication root, callable code in persisted descriptors, active-but-unenforced constraints, new DDL grammar without profile ownership or complete expression-index execution yet.

## Bridge/deletion boundary

Any lower registry retained is a derived runtime view with a named owner: constraints/index execution F05-PR05/06; native registry integration F04-PR06/08.

## Standards and reviewer focus

§4.2.5 catalog; §4.13 graph types; native constraints/index declarations are implementation facilities unless specifically standardized.

**Independent review question:** Can every usable runtime registration be explained by one catalog declaration and the current graph/profile generation?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
