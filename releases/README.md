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
