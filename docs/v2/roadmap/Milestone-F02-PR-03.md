---
plan_id: F02-PR03
milestone: F02
initial_status: proposed
---

# F02-PR03 — Encode complete logical transactions in the format-2 WAL

**Milestone:** [F02: Bring durable embedding forward](Milestone-F02.md)
**Dependencies:** [F02-PR01](Milestone-F02-PR-01.md), [F02-PR02](Milestone-F02-PR-02.md), [F01-PR03](Milestone-F01-PR-03.md), [F03-PR02](Milestone-F03-PR-02.md)
**Carries forward:** M09-PR04; see the [complete crosswalk](06-OLD-TO-NEW-MAP.md).
**Issue closure:** None; do not close another PR’s issue.
**Focused skills:** `rust-storage-durability`; `rust-protocol-codecs`; `rust-test-design` from `yet-more-skills/skills/<name>/SKILL.md`.

## Outcome

Provide a bounded, versioned logical transaction codec that can replay catalog and graph changes as one unit without persisting physical layouts or runtime handles.

## Start from what exists

The current Value re-export is a documented bridge with bare-ID reference variants. F03-PR02 must settle runtime versus stored values before this codec is treated as format 2. Mixed edges and catalog metadata must also be present; persistence cannot encode only the easy graph mutations. Source: S05.

**Observed live entry points:** `crates/selene-persist/src/payload.rs`, `crates/selene-persist/src/entry_header.rs`, `crates/selene-persist/src/file_header.rs`, `crates/selene-persist/src/reader.rs`, `crates/selene-persist/src/recovery.rs`, `crates/selene-core/src`

Paths are navigation, not a closed edit inventory. New modules are implementation choices. Keep required callers and their tests with the behavior they support. Source IDs resolve in [SOURCE-NOTES.md](SOURCE-NOTES.md).

## Implementation sequence

1. Inventory all authoritative transaction payloads: catalog/type/declaration changes, node/edge creation and deletion, properties, directionality, stable endpoints and allocator high-water data required to avoid reusing published IDs.
2. Define explicit framing with version, lengths, sequence, integrity protection and lineage. Use checked arithmetic and hard resource bounds before allocating or decompressing attacker-controlled lengths.
3. Encode stored semantic values through the durable value boundary from F03-PR02. Reject ephemeral candidates, execution contexts and process-local handles. If persistent references are in the selected property profile, encode durable store/graph/element identity and validate their referents; otherwise reject them as stored values.
4. Use one logical transaction envelope for all mutations in its atomic unit. Decode and validate the entire record before applying it to isolated recovery state; an invalid final field must not leave half a transaction applied.
5. Provide independently constructed valid/invalid framing vectors as well as round trips. Make unknown authoritative kinds fatal; only explicitly non-authoritative extension data may be skipped.

## Acceptance and concrete regression cases

- [ ] A catalog change and mutations in every graph touched by the transaction recover together or not at all.
- [ ] Mixed edges, deleted IDs, empty/null values and selected numeric/temporal representations survive independent decode fixtures.
- [ ] Oversized, overflowing, nested, truncated, compressed-bomb and unknown-version inputs fail within configured work/memory bounds.
- [ ] A missing sequence, foreign StoreId/epoch or invalid referential order fails before recovery publication.
- [ ] An incomplete final unsealed frame is distinguishable from an interior or sealed corruption; checksum failure is not automatically a harmless tail.
- [ ] The on-disk schema contains no Rust enum layout, pointer, candidate token or graph storage-row coordinate.

## Validation and performance

Run decoder property tests, fixed golden vectors and logical transaction apply tests. Add a short targeted fuzz smoke for changed decoders; long campaigns belong to F02-PR07/RC. Round-trip equality alone is not format correctness.

Measure encoded bytes, encode/decode allocation, transactions per second and maximum-input rejection cost. Record durable format overhead separately from later fsync cost.

Use the shared [validation guide](05-VALIDATION-AND-RELEASE.md) for runner mechanics and required PR/RC gates. These are planned checks, not reported passes.

## Keep out of this PR

No format-1 reader, arbitrary recovery salvage, heuristic skip of unknown authoritative records, unsafe zero-copy casting or serialization of the facade DatabaseId.

## Bridge/deletion boundary

Legacy codecs remain unreachable from new format-2 entry points and are deleted by F02-PR08. Keep the formats visibly separate during the transition.

## Standards and reviewer focus

§4.6 and §8 atomicity; §4.4 values and references. Byte framing and integrity mechanism are Selene implementation choices.

**Independent review question:** Can malformed input cause allocation before bounds checks or partially apply an authoritative transaction?

Use [Luna execution guidance](03-LUNA-EXECUTION.md) for material decisions and the compact handoff. A necessary adjacent caller or mechanical migration is not itself a reason to stop; an incompatible semantic, public or durable contract is.
