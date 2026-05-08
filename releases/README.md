# Release Notes

Top-level release notes live here as `releases/<version>.md`.

Each release note aggregates per-crate changelog entries and adds the cross-crate
context needed by users upgrading a full selene-db workspace.

## Pending Pre-Release Notes

- BRIEF-03: declared transaction model is single-writer + MVCC; ID allocation produces permanent holes on abort; WAL carries opaque caller principal slot.
- BRIEF-04a: snapshot section tags are 8 bytes (provider+sub-tag); imbl-shaped state archives via rkyv sorted-vec intermediate; recovery is two-step (snapshot apply + WAL replay) with the canonical IndexProvider trait in spec 06.
- BRIEF-04b: extension API surface hardened — single ProcedureRegistry trait in selene-gql; per-tier concrete Context structs with dyn-compatible Procedure traits; pack-lifecycle audit consolidates through WAL only (AuditEntry/EMITTED_AUDIT removed).
- BRIEF-05: selene-core foundation bootstrapped with primitive IDs, IStr interning, Value variants, extension type IDs, and ValueTypeAdapter registry.
- BRIEF-05.1: IStr cap admission is now atomic under concurrency (double-checked locking with a static admission Mutex); fixes PR #3 Copilot P1 finding.
- BRIEF-INFRA-01: CI speedup — cargo-audit and cargo-about install via prebuilt binaries (taiki-e/install-action); advisory DB cached; concurrency block cancels obsolete runs.
- BRIEF-06: selene-core composites completed with PropertyMap, LabelSet, schema model, transient Codec, Origin, Change/SchemaChange payloads, serde/postcard transit derives, and spec 02 amendments for adapter validation + Codec framing.
- BRIEF-07: selene-graph foundation bootstrapped with chunked SoA storage, lock-free snapshots, serialized write transactions, SharedGraph-level ID allocation, and the Mutator change funnel; indexes/schema validation remain deferred.
- BRIEF-08: selene-graph added built-in node/edge label indexes, the `IndexProvider` extension trait, provider registration/lookup, and log-and-continue provider notification on commit.
- BRIEF-09: selene-graph added built-in `TypedIndex` node property indexes with strict registration, mutation-funnel maintenance, read accessors, and a six-kind v1.0 index surface.
- BRIEF-10: selene-persist bootstrapped the v1.0 single-file WAL (`wal.log`) with SLDB headers, postcard/zstd payloads, blake3-low-32 checksums, HLC/origin/principal audit headers, writer append/group-commit, and lazy reader iteration.
- BRIEF-11: selene-persist added the v1.0 snapshot envelope (`snapshot.{seq}.snap`) with SLSN headers, TLV section tables, atomic tmp-file publication, blake3-128 body hashes, per-section zstd compression, bounded reads, and latest-snapshot path helpers.
- BRIEF-12: selene-persist added the recovery API with `RecoveryProvider`, deterministic `ProviderRegistry`, snapshot apply, WAL replay, WAL/snapshot epoch validation, provider error wrapping, and per-entry replicated-change dedupe.
- BRIEF-INFRA-02: selene-core added symbolic registries for first-party `ExtensionTypeId` rendering and emitted GQLSTATUS code names, with drift tests in selene-core, selene-graph, and selene-persist.
- BRIEF-13: selene-graph added the auto-registered CORE provider, postcard snapshot sections for primary graph state, SharedGraph recovery through selene-persist, and cold-start rebuild of adjacency plus secondary indexes.
- BRIEF-14: selene-graph switched CORE snapshot sections from postcard payloads to rkyv sorted-vec archives, with portable UTF-8 `IStr` archiving and postcard retained only inside per-row property blobs.
