---
plan_id: F03-PR02
milestone: F03
initial_status: proposed
---

# F03-PR02 — Unify structural types, values and reference boundaries

**Milestone:** [F03: Complete the semantic compiler](Milestone-F03.md)
**Dependencies:** [F03-PR01](Milestone-F03-PR-01.md), [F01-PR02](Milestone-F01-PR-02.md)
**Carries forward:** M05-PR03; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-api-design`; `rust-test-design`; `rust-storage-durability` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Provide one structural type service and an explicit runtime/stored-value boundary so analysis, results and persistence agree without exposing lower bare-ID reference carriers.

## Start from what exists

The live facade re-exports selene_core::Value and selene_gql::GqlType as temporary bridges. Its database-scoped GraphRef/NodeRef/EdgeRef are different types from the lower bare-ID Value variants. This is a real integration prerequisite for the format-2 codec, not cosmetic API cleanup. Source: S05.

**Observed live entry points:** `crates/selene-db/src/lib.rs`, `crates/selene-db/src/params.rs`, `crates/selene-db/src/outcome.rs`, `crates/selene-db/src/handle.rs`, `crates/selene-core/src`, `crates/selene-gql/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Inventory the selected profile and existing value families before defining descriptors. Reuse correct scalar representations; add normalized structural descriptors for nullability, records/lists, graph references, paths and open/dynamic types actually selected.
2. Define separate operations for assignment, comparability, equality, distinctness, grouping and ordering. Do not force them through one Rust Eq/Hash/Ord implementation when language semantics differ.
3. Keep omitted result, empty binding table and null value distinct. Result schemas retain field names/types, ordering metadata and preferred output column order even when no rows exist.
4. Migrate parameters/results/runtime references away from ambiguous lower bare-ID carriers. Validate database/graph provenance when converting external handles. Persisted values use a separate explicit codec-facing contract; intern IDs and process-local DatabaseId are not durable data.
5. Replace facade GqlType/Value compatibility exposure with intentionally supported facade types or documented stable re-exports. Migrate all existing runtime consumers needed to make that boundary real; old byte codecs are only temporary internal debt until F02-PR08.

## Acceptance and concrete regression cases

- [ ] Null comparison produces Unknown where required, but two nulls are not distinct for duplicate/grouping purposes; omitted result is neither of these.
- [ ] Equivalent selected structural type descriptions normalize identically; unsupported types fail rather than widening to an untyped catch-all.
- [ ] Record field names, list element types, reference provenance and nullable variants survive parameters → execution → typed result.
- [ ] Foreign handles with matching numeric IDs are rejected, and deleted referents produce the required invalid-reference diagnostic when accessed.
- [ ] Numeric equality/grouping/hash compatibility covers mixed selected exact numeric forms, signed zero and documented floating-point edge cases.
- [ ] Stored-value tests reject ephemeral references/candidates or encode only explicitly supported durable semantic reference forms; no descriptor arena index becomes an on-disk type ID.

## Validation and performance

Run pure type-model tests plus facade parameter/result, GQL expression and reference tests. Use generated selected-profile inventories to ensure every supported value family has a case. Keep structural normalization snapshots as regression evidence, not an independent proof of semantic correctness.

Measure descriptor lookup/interning, conversion allocation and common scalar evaluation. Avoid global intern pools that keep database-owned types alive indefinitely.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No all-optional-type expansion, universal comparison feature by accident, typed errors collapsed to null, byte-format design before this boundary or compatibility aliases that preserve the ambiguity.

## Bridge/deletion boundary

The public lower Value/GqlType bridge ends here. Old encoded variants may remain only in isolated legacy persistence code until F02-PR08; temporary semantic lowering is deleted in F03-PR04.

## Standards and reviewer focus

§§4.4, 4.12–4.17; §18.9; §19; §20; §§22.11–22.20. Check the exact selected numeric and reference rules, not host-language defaults.

**Independent review question:** Can the runtime, compiler and durable codec disagree about what a value means even though their Rust types compile?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
